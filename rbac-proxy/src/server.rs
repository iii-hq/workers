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

use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::extract::ConnectInfo;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Extension, Router};
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use iii_sdk::IIIClient;
use tokio::net::TcpListener;
use tokio::sync::{watch, Mutex, RwLock};

use crate::configuration::ConfigCell;
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
    Router::new()
        .route("/", get(proxy::ws_upgrade))
        .route("/ws/channels/:channel_id", get(channels::channel_bridge))
        .fallback(not_found)
        .with_state(state)
}

async fn not_found() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "not found")
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
                        Err(rollback) => return Err(format!("replacement failed: {error:#}; restoring previous listener failed: {rollback:#}")),
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
}
