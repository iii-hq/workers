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

use std::sync::atomic::AtomicU32;
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
/// live accept loop; [`rebind`](ServerHandle::rebind) binds a new
/// listener (last-good on failure) and gracefully retires the old one.
pub struct ServerHandle {
    state: ProxyState,
    /// Graceful-shutdown trigger for the live server (replaced on rebind).
    current: Mutex<Option<watch::Sender<bool>>>,
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
        })
    }

    /// Rebind to a new `host:port`. Binds the **new** listener first; on
    /// success swaps it in and gracefully retires the old server (existing
    /// connections drain on the old listener). On **bind failure** the
    /// previous listener is kept (last-good) and the error is returned so the
    /// caller can also keep the previous config.
    pub async fn rebind(&self, host: &str, port: u16) -> Result<(), String> {
        let listener = bind(host, port)
            .await
            .map_err(|e| format!("binding {host}:{port}: {e}"))?;
        let addr = listener
            .local_addr()
            .map_err(|e| format!("reading local_addr: {e}"))?;

        let tx = spawn_server(listener, self.state.clone());
        let mut cur = self.current.lock().await;
        if let Some(old) = cur.take() {
            let _ = old.send(true); // graceful: stop accepting, drain in-flight
        }
        *cur = Some(tx);
        *self.state.bound.write().await = (host.to_string(), addr.port());
        tracing::info!(addr = %addr, "rbac-proxy listener rebound");
        Ok(())
    }

    /// Gracefully stop the live server (used on process shutdown).
    pub async fn shutdown(&self) {
        if let Some(tx) = self.current.lock().await.take() {
            let _ = tx.send(true);
        }
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
/// trigger. The `JoinHandle` is intentionally detached: when the trigger
/// fires the loop stops accepting and tells every connection in flight to
/// shut down gracefully.
fn spawn_server(listener: TcpListener, state: ProxyState) -> watch::Sender<bool> {
    serve_with_deadline(listener, state, HEADER_READ_TIMEOUT)
}

/// What `axum::serve` does (HTTP/1 with upgrades, the peer address as
/// `ConnectInfo`), plus a deadline on reading the request headers.
fn serve_with_deadline(
    listener: TcpListener,
    state: ProxyState,
    header_timeout: Duration,
) -> watch::Sender<bool> {
    let (tx, mut rx) = watch::channel(false);
    let app = router(state);
    tokio::spawn(async move {
        // Once the loop stops: idle keep-alive connections close at once, the
        // rest after their response (the engine listener's shutdown).
        let (conn_shutdown_tx, _) = watch::channel(false);
        loop {
            let (stream, peer) = tokio::select! {
                accepted = listener.accept() => match accepted {
                    Ok(conn) => conn,
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
    tx
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

    async fn serve(header_timeout: Duration) -> (watch::Sender<bool>, SocketAddr) {
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
        let _ = tx.send(true);
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
        let _ = tx.send(true);
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
