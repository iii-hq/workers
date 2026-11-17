//! The public RBAC listener: the axum router (worker-protocol route `/` plus
//! the channel bridge `/ws/channels/{id}`) and a **rebindable** server so a
//! `host`/`port` hot reload can swap the listener without restarting the
//! process.
//!
//! The router state ([`ProxyState`]) is shared by every connection: the live
//! [`ConfigCell`](crate::configuration::ConfigCell) snapshot, the control
//! connection, the discovery [`CatalogCache`](crate::engine_overrides::
//! CatalogCache), the live-connection gauge, and the actually-bound address
//! (the source of truth `rbac-proxy::status` reports).

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use anyhow::Context;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Router};
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use iii_sdk::IIIClient;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, RwLock, watch};

use crate::configuration::ConfigCell;
use crate::endpoints::{self, Endpoint};
use crate::engine_overrides::CatalogCache;
use crate::{channels, proxy};

/// Shared runtime state for every connection (all fields are cheap `Arc`
/// clones).
#[derive(Clone)]
pub struct ProxyState {
    /// Live config snapshot (hot-reloaded); read per upgrade.
    pub config: ConfigCell,
    /// Control connection — auth/middleware/hook calls and the discovery caches.
    pub iii: Arc<IIIClient>,
    /// Discovery catalog + binding caches over the control connection.
    pub catalog: Arc<CatalogCache>,
    /// Live downstream connections.
    pub active: Arc<AtomicU32>,
    /// The actually-bound `(host, port)` — updated on each successful bind,
    /// kept last-good on a failed rebind.
    pub bound: Arc<RwLock<(String, u16)>>,
}

impl ProxyState {
    pub fn new(config: ConfigCell, iii: Arc<IIIClient>) -> Self {
        let catalog = Arc::new(CatalogCache::new(iii.clone()));
        Self {
            config,
            iii,
            catalog,
            active: Arc::new(AtomicU32::new(0)),
            bound: Arc::new(RwLock::new((String::new(), 0))),
        }
    }
}

/// Build the router. Exposed for tests so they can drive it without a socket.
pub fn router(state: ProxyState) -> Router {
    Router::new().fallback(endpoint).with_state(state)
}

/// Every request: a 404 unless it is one of the engine's endpoints
/// ([`Endpoint::of`]) and `rbac.expose_endpoints` exposes it, then handled
/// as that endpoint. The 404 says nothing more: what is exposed, and how to
/// expose more, is for the operator's log, not for whoever is probing. Rules
/// are read per request, so a configuration change applies to the next
/// connection.
async fn endpoint(
    State(state): State<ProxyState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    uri: Uri,
    headers: HeaderMap,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let path = uri.path().to_string();
    let Some(kind) = Endpoint::of(&path) else {
        return not_found();
    };
    let cfg = state.config.read().await.clone();
    if !endpoints::endpoint_exposed(&cfg.rbac.endpoints(), &path) {
        tracing::debug!(hint = %endpoints::not_exposed_hint(&path), "refused an unexposed endpoint");
        return not_found();
    }
    let Ok(ws) = ws else {
        return (
            StatusCode::UPGRADE_REQUIRED,
            "this endpoint takes a WebSocket upgrade",
        )
            .into_response();
    };
    match kind {
        Endpoint::Worker => {
            ws.on_upgrade(move |socket| proxy::handle_connection(socket, state, peer, uri, headers))
        }
        Endpoint::Channel(_) => {
            let query = uri.query().map(str::to_string);
            ws.on_upgrade(move |socket| {
                channels::channel(socket, state, cfg.engine_url.clone(), path, query)
            })
        }
        Endpoint::Telemetry => {
            // Authenticated before the upgrade: a refusal is an HTTP status,
            // which an SDK backs off from, not a 101 and an error frame,
            // which resets its reconnect backoff.
            if let Err(rej) = crate::rbac::resolve_session(
                &state.iii,
                Some(&cfg.rbac),
                proxy::headers_to_map(&headers),
                proxy::query_to_multi_map(uri.query()),
                peer.ip().to_string(),
                &path,
            )
            .await
            {
                tracing::info!(code = %rej.code, reason = %rej.message, "refused /otel");
                return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
            }
            ws.on_upgrade(move |socket| proxy::telemetry_relay(socket, state, uri))
        }
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

/// A rebindable public listener. Holds the shutdown trigger for the currently
/// live accept loop; [`rebind`](ServerHandle::rebind) stages replacements
/// where possible and rolls back overlapping same-port replacements on failure.
pub struct ServerHandle {
    state: ProxyState,
    /// Graceful-shutdown trigger for the live server (replaced on rebind).
    current: Mutex<Option<RunningServer>>,
    pub(crate) reload: Mutex<()>,
    closed: AtomicBool,
}

impl ServerHandle {
    /// Bind `host:port` and start serving. Fatal on failure (boot dependency).
    pub async fn bind_and_serve(state: ProxyState, host: &str, port: u16) -> anyhow::Result<Self> {
        let listener = bind(host, port).await?;
        let addr = listener.local_addr().context("reading bound local_addr")?;
        *state.bound.write().await = (host.to_string(), addr.port());
        tracing::info!(addr = %addr, "rbac-proxy public listener bound");
        let tx = spawn_server(listener, state.clone());
        Ok(Self {
            state,
            current: Mutex::new(Some(tx)),
            reload: Mutex::new(()),
            closed: AtomicBool::new(false),
        })
    }

    /// Stage non-overlapping binds. Same-port wildcard changes must release
    /// the old socket first; a failed replacement restores the previous bind.
    pub async fn rebind(&self, host: &str, port: u16) -> Result<(), String> {
        let mut cur = self.current.lock().await;
        if self.closed.load(Ordering::Relaxed) {
            return Err("proxy server is shut down".to_string());
        }
        let previous = self.state.bound.read().await.clone();
        let overlapping = cur.is_some() && overlaps(&previous, host, port);
        if overlapping {
            cur.take().unwrap().stop().await;
            *self.state.bound.write().await = (String::new(), 0);
        }
        let listener = match bind(host, port).await {
            Ok(listener) => listener,
            Err(error) => {
                if overlapping {
                    match bind(&previous.0, previous.1).await {
                        Ok(listener) => {
                            *cur = Some(spawn_server(listener, self.state.clone()));
                            *self.state.bound.write().await = previous;
                        }
                        Err(rollback) => {
                            return Err(format!(
                                "replacement failed: {error:#}; restoring previous listener failed: {rollback:#}"
                            ));
                        }
                    }
                }
                return Err(format!("binding {host}:{port}: {error:#}"));
            }
        };
        let addr = listener
            .local_addr()
            .map_err(|e| format!("reading local_addr: {e}"))?;
        let next = spawn_server(listener, self.state.clone());
        if let Some(old) = cur.replace(next) {
            old.stop().await;
        }
        *self.state.bound.write().await = (host.to_string(), addr.port());
        tracing::info!(addr = %addr, "rbac-proxy listener rebound");
        Ok(())
    }

    pub async fn bound_address(&self) -> (String, u16) {
        self.state.bound.read().await.clone()
    }

    /// Stop accepting and await release of the listening socket.
    pub async fn shutdown(&self) {
        let _reload = self.reload.lock().await;
        let mut current = self.current.lock().await;
        self.closed.store(true, Ordering::Relaxed);
        if let Some(server) = current.take() {
            server.stop().await;
        }
        *self.state.bound.write().await = (String::new(), 0);
    }
}

fn overlaps(previous: &(String, u16), host: &str, port: u16) -> bool {
    if previous.1 != port || port == 0 || previous.0 == host {
        return false;
    }
    match (previous.0.parse::<IpAddr>(), host.parse::<IpAddr>()) {
        (Ok(old), Ok(new)) => {
            old.is_ipv4() == new.is_ipv4() && (old.is_unspecified() || new.is_unspecified())
        }
        _ => false,
    }
}

struct RunningServer {
    shutdown: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}
impl RunningServer {
    async fn stop(self) {
        let _ = self.shutdown.send(true);
        let _ = self.task.await;
    }
}

async fn bind(host: &str, port: u16) -> anyhow::Result<TcpListener> {
    TcpListener::bind((host, port))
        .await
        .with_context(|| format!("binding TCP listener on {host}:{port}"))
}

/// How long a client may take to send its request headers before the
/// connection is closed. `axum::serve` sets no timer, so a socket that never
/// finished its upgrade stayed open for good; the engine's own RBAC listener
/// closed it after `handshake_timeout_ms` (10 s by default).
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Spawn the accept loop on `listener`, returning its graceful-shutdown
/// trigger and task handle, so a replacement can await listening-socket release.
/// Existing HTTP connections shut down gracefully; upgraded sessions continue.
fn spawn_server(listener: TcpListener, state: ProxyState) -> RunningServer {
    serve_with_deadline(listener, state, HEADER_READ_TIMEOUT)
}

/// What `axum::serve` does (HTTP/1 with upgrades, the peer address as
/// `ConnectInfo`), plus a deadline on reading the request headers.
fn serve_with_deadline(
    listener: TcpListener,
    state: ProxyState,
    header_timeout: Duration,
) -> RunningServer {
    let (tx, mut rx) = watch::channel(false);
    let app = router(state);
    let task = tokio::spawn(async move {
        // Once the loop stops: idle keep-alive connections close at once, the
        // rest after their response (the engine listener's shutdown).
        let (conn_shutdown_tx, _) = watch::channel(false);
        loop {
            let (stream, peer) = tokio::select! {
                accepted = listener.accept() => match accepted {
                    Ok((stream, peer)) => {
                        // Small frames must not wait for the peer's delayed ACK
                        // (the engine listener does the same, MOT-5377).
                        if let Err(e) = stream.set_nodelay(true) {
                            tracing::debug!(error = %e, "could not set TCP_NODELAY");
                        }
                        (stream, peer)
                    }
                    Err(e) if matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionRefused
                            | std::io::ErrorKind::ConnectionAborted
                            | std::io::ErrorKind::ConnectionReset
                    ) => continue,
                    Err(e) => {
                        // Out of file descriptors and the like: wait, as axum does.
                        tracing::error!(error = %e, "rbac-proxy accept failed");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                },
                changed = rx.changed() => {
                    if changed.is_err() || *rx.borrow() {
                        break;
                    }
                    continue;
                }
            };
            let service = TowerToHyperService::new(app.clone().layer(Extension(ConnectInfo(peer))));
            let mut conn_shutdown_rx = conn_shutdown_tx.subscribe();
            tokio::spawn(async move {
                let conn = http1::Builder::new()
                    .timer(TokioTimer::new())
                    .header_read_timeout(header_timeout)
                    .serve_connection(TokioIo::new(stream), service)
                    .with_upgrades();
                tokio::pin!(conn);
                // Completes on the shutdown send, or when the sender goes.
                let conn_shutdown = async move {
                    while conn_shutdown_rx.changed().await.is_ok() {
                        if *conn_shutdown_rx.borrow() {
                            break;
                        }
                    }
                };
                tokio::pin!(conn_shutdown);
                let mut graceful_sent = false;
                loop {
                    tokio::select! {
                        served = conn.as_mut() => {
                            match served {
                                Err(e) if e.is_timeout() => tracing::warn!(
                                    peer = %peer,
                                    timeout_ms = header_timeout.as_millis() as u64,
                                    "closed socket: request headers not received within the deadline"
                                ),
                                Err(e) => tracing::debug!(peer = %peer, error = %e, "connection ended"),
                                Ok(()) => {}
                            }
                            break;
                        }
                        _ = &mut conn_shutdown, if !graceful_sent => {
                            conn.as_mut().graceful_shutdown();
                            graceful_sent = true;
                        }
                    }
                }
            });
        }
        let _ = conn_shutdown_tx.send(true);
    });
    RunningServer { shutdown: tx, task }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkerConfig;
    use std::net::SocketAddr;

    fn state() -> ProxyState {
        let iii = Arc::new(iii_sdk::register_worker(
            "ws://127.0.0.1:1",
            iii_sdk::InitOptions::default(),
        ));
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        ProxyState::new(cell, iii)
    }

    #[tokio::test]
    async fn bind_and_rebind_updates_bound_addr() {
        // Bind to an ephemeral port (port 0) so the test never collides.
        let h = ServerHandle::bind_and_serve(state(), "127.0.0.1", 0)
            .await
            .expect("initial bind");
        let (host1, port1) = h.state.bound.read().await.clone();
        assert_eq!(host1, "127.0.0.1");
        assert!(port1 > 0, "an ephemeral port was assigned");

        // Rebind to another ephemeral port; bound addr updates.
        h.rebind("127.0.0.1", 0).await.expect("rebind");
        let (_h2, port2) = h.state.bound.read().await.clone();
        assert!(port2 > 0);

        h.shutdown().await;
    }

    async fn serve(header_timeout: Duration) -> (RunningServer, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        (serve_with_deadline(listener, state(), header_timeout), addr)
    }

    async fn serve_exposing(patterns: Option<&[&str]>) -> (RunningServer, SocketAddr) {
        let mut cfg = WorkerConfig::default();
        cfg.rbac.expose_endpoints = patterns.map(|patterns| {
            serde_json::from_value(serde_json::json!(
                patterns
                    .iter()
                    .map(|p| format!("match(\"{p}\")"))
                    .collect::<Vec<_>>()
            ))
            .unwrap()
        });
        let iii = Arc::new(iii_sdk::register_worker(
            "ws://127.0.0.1:1",
            iii_sdk::InitOptions::default(),
        ));
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg)));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = ProxyState::new(cell, iii);
        (
            serve_with_deadline(listener, state, Duration::from_secs(10)),
            addr,
        )
    }

    /// The response head and body to a GET of `path`, as a WebSocket upgrade
    /// or a plain request.
    async fn get(addr: SocketAddr, path: &str, upgrade: bool) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        let upgrade_headers = if upgrade {
            "Upgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n"
        } else {
            "Connection: close\r\n"
        };
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n{upgrade_headers}\r\n").as_bytes())
            .await
            .unwrap();
        let mut buf = vec![0u8; 4096];
        let n = tokio::time::timeout(Duration::from_secs(3), s.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        String::from_utf8_lossy(&buf[..n]).to_string()
    }

    #[tokio::test]
    async fn no_endpoint_is_served_from_an_empty_list() {
        let (tx, addr) = serve_exposing(Some(&[])).await;
        for path in ["/", "/otel", "/ws/channels/abc", "/nope"] {
            for upgrade in [true, false] {
                let res = get(addr, path, upgrade).await;
                assert!(res.starts_with("HTTP/1.1 404"), "{path}: {res}");
            }
        }
        tx.stop().await;
    }

    #[tokio::test]
    async fn a_refusal_names_neither_the_product_nor_the_setting() {
        let (tx, addr) = serve_exposing(Some(&["/"])).await;
        for path in ["/admin", "/.env", "/otel"] {
            let res = get(addr, path, false).await;
            assert!(res.starts_with("HTTP/1.1 404"), "{path}: {res}");
            for leak in ["rbac", "expose_endpoints", path] {
                assert!(
                    !res.split("\r\n\r\n").nth(1).unwrap_or("").contains(leak),
                    "{path} leaks {leak}: {res}"
                );
            }
        }
        tx.stop().await;
    }

    #[tokio::test]
    async fn a_missing_key_serves_only_the_worker_endpoint() {
        let (tx, addr) = serve_exposing(None).await;
        assert!(get(addr, "/", true).await.starts_with("HTTP/1.1 101"));
        assert!(get(addr, "/otel", true).await.starts_with("HTTP/1.1 404"));
        assert!(
            get(addr, "/ws/channels/abc", true)
                .await
                .starts_with("HTTP/1.1 404")
        );
        tx.stop().await;
    }

    #[tokio::test]
    async fn exposed_endpoints_upgrade_and_unknown_paths_never_do() {
        let (tx, addr) = serve_exposing(Some(&["/", "/ws/channels/*", "*"])).await;
        for path in ["/", "/ws/channels/abc"] {
            let res = get(addr, path, true).await;
            assert!(res.starts_with("HTTP/1.1 101"), "{path}: {res}");
        }
        // `*` matches them, but they are not engine endpoints.
        for path in [
            "/nope",
            "/ws/channels/a/b",
            "/ws/channels/..",
            "/otel/v2",
            "//",
        ] {
            let res = get(addr, path, true).await;
            assert!(res.starts_with("HTTP/1.1 404"), "{path}: {res}");
        }
        tx.stop().await;
    }

    #[tokio::test]
    async fn otel_authenticates_before_the_upgrade() {
        // No auth function: refused. The refusal is an HTTP status, never a
        // 101 followed by an error frame, so an SDK backs off.
        let (tx, addr) = serve_exposing(Some(&["/otel"])).await;
        let res = get(addr, "/otel", true).await;
        assert!(res.starts_with("HTTP/1.1 401"), "{res}");
        tx.stop().await;
    }

    #[tokio::test]
    async fn an_exposed_endpoint_still_needs_a_websocket() {
        let (tx, addr) = serve_exposing(Some(&["/"])).await;
        let res = get(addr, "/", false).await;
        assert!(res.starts_with("HTTP/1.1 426"), "{res}");
        tx.stop().await;
    }

    #[tokio::test]
    async fn stalled_headers_are_cut_off() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (tx, addr) = serve(Duration::from_millis(200)).await;
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        // Headers that never end.
        s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n").await.unwrap();
        let mut buf = Vec::new();
        let closed = tokio::time::timeout(Duration::from_secs(3), s.read_to_end(&mut buf)).await;
        assert!(
            closed.is_ok(),
            "a connection whose headers never ended stayed open"
        );
        tx.stop().await;
    }

    /// The `{channel_id}` route builds under axum 0.8 syntax, upgrades, and
    /// reports a failed engine dial to the client as close 1011 with reason.
    #[tokio::test]
    async fn channel_route_upgrades_and_reports_dial_failure() {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::Message;
        let iii = Arc::new(iii_sdk::register_worker(
            "ws://127.0.0.1:1",
            iii_sdk::InitOptions::default(),
        ));
        let mut config = WorkerConfig {
            engine_url: "ws://127.0.0.1:1".to_string(),
            ..WorkerConfig::default()
        };
        // Nothing is served until exposed.
        config.rbac.expose_endpoints =
            Some(serde_json::from_value(serde_json::json!(["match(\"/ws/channels/*\")"])).unwrap());
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(config)));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let tx = serve_with_deadline(
            listener,
            ProxyState::new(cell, iii),
            Duration::from_secs(10),
        );

        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/channels/ch-1"))
            .await
            .expect("the channel route upgrades");
        let frame = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("a frame before the timeout")
            .expect("a frame")
            .expect("a well-formed frame");
        let Message::Close(Some(close)) = frame else {
            panic!("expected a close frame, got {frame:?}");
        };
        assert_eq!(u16::from(close.code), 1011);
        assert_eq!(close.reason.as_str(), "engine channel dial failed");
        tx.stop().await;
    }

    #[tokio::test]
    async fn requests_still_reach_the_router_with_the_peer() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (tx, addr) = serve(Duration::from_secs(10)).await;
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), s.read_to_end(&mut buf))
            .await
            .unwrap()
            .unwrap();
        let head = String::from_utf8_lossy(&buf);
        // ws_upgrade reads ConnectInfo before WebSocketUpgrade: a missing peer is a 500.
        assert!(head.starts_with("HTTP/1.1 4"), "{head}");
        tx.stop().await;
    }

    #[tokio::test]
    async fn running_default_wildcard_listener_can_narrow_to_loopback() {
        let h = ServerHandle::bind_and_serve(state(), "0.0.0.0", 0)
            .await
            .unwrap();
        let port = h.state.bound.read().await.1;
        h.rebind("127.0.0.1", port)
            .await
            .expect("claim default wildcard listener on the same port");
        assert_eq!(*h.state.bound.read().await, ("127.0.0.1".to_string(), port));
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        h.shutdown().await;
    }

    #[tokio::test]
    async fn failed_overlapping_bind_restores_wildcard_listener() {
        let h = ServerHandle::bind_and_serve(state(), "0.0.0.0", 0)
            .await
            .unwrap();
        let before = h.bound_address().await;
        assert!(h.rebind("203.0.113.1", before.1).await.is_err());
        assert_eq!(h.bound_address().await, before);
        tokio::net::TcpStream::connect(("127.0.0.1", before.1))
            .await
            .unwrap();
        h.rebind("127.0.0.1", before.1).await.unwrap();
        h.shutdown().await;
    }

    #[tokio::test]
    async fn rebind_failure_keeps_last_good() {
        let h = ServerHandle::bind_and_serve(state(), "127.0.0.1", 0)
            .await
            .expect("initial bind");
        let before = h.state.bound.read().await.clone();

        // An unroutable host fails to bind; last-good is preserved.
        let err = h.rebind("203.0.113.1", 0).await;
        assert!(err.is_err(), "binding an unassigned address should fail");
        let after = h.state.bound.read().await.clone();
        assert_eq!(before, after, "bound addr unchanged on failed rebind");

        h.shutdown().await;
    }

    /// A WebSocket "engine" that echoes every frame, for relay tests.
    async fn echo_engine() -> SocketAddr {
        use futures_util::{SinkExt, StreamExt};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let Ok(mut ws) = tokio_tungstenite::accept_async(tcp).await else {
                        return;
                    };
                    while let Some(Ok(m)) = ws.next().await {
                        if m.is_close() || ws.send(m).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
        addr
    }

    async fn relay_fixture() -> (RunningServer, SocketAddr, ProxyState, ConfigCell) {
        let engine = echo_engine().await;
        let mut config = WorkerConfig {
            engine_url: format!("ws://{engine}"),
            ..WorkerConfig::default()
        };
        config.rbac.expose_endpoints =
            Some(serde_json::from_value(serde_json::json!(["match(\"/ws/channels/*\")"])).unwrap());
        let iii = Arc::new(iii_sdk::register_worker(
            "ws://127.0.0.1:1",
            iii_sdk::InitOptions::default(),
        ));
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(config)));
        let state = ProxyState::new(cell.clone(), iii);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let tx = serve_with_deadline(listener, state.clone(), Duration::from_secs(10));
        (tx, addr, state, cell)
    }

    async fn eventually(mut check: impl FnMut() -> bool) -> bool {
        for _ in 0..100 {
            if check() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    #[tokio::test]
    async fn relayed_sockets_count_as_active_connections() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let (tx, addr, state, _cell) = relay_fixture().await;
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/channels/c1"))
            .await
            .unwrap();
        ws.send(Message::Text("ping".into())).await.unwrap();
        let echoed = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .unwrap();
        assert_eq!(echoed.unwrap().unwrap(), Message::Text("ping".into()));
        assert_eq!(state.active.load(Ordering::Relaxed), 1);
        ws.close(None).await.unwrap();
        assert!(eventually(|| state.active.load(Ordering::Relaxed) == 0).await);
        tx.stop().await;
    }

    #[tokio::test]
    async fn a_reload_that_stops_exposing_a_path_closes_its_relays() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let (tx, addr, state, cell) = relay_fixture().await;
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/channels/c1"))
            .await
            .unwrap();
        ws.send(Message::Text("ping".into())).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .unwrap();

        // A reload that keeps the path exposed leaves the relay alone.
        let same = (**cell.read().await).clone();
        crate::configuration::apply_config(&cell, same).await;
        ws.send(Message::Text("still".into())).await.unwrap();
        let echoed = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .unwrap();
        assert_eq!(echoed.unwrap().unwrap(), Message::Text("still".into()));

        let mut closed = (**cell.read().await).clone();
        closed.rbac.expose_endpoints = Some(vec![]);
        crate::configuration::apply_config(&cell, closed).await;
        let frame = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("closed before the timeout");
        let Some(Ok(Message::Close(Some(close)))) = frame else {
            panic!("expected a close frame, got {frame:?}");
        };
        assert_eq!(u16::from(close.code), 1008);
        assert!(eventually(|| state.active.load(Ordering::Relaxed) == 0).await);
        tx.stop().await;
    }
}
