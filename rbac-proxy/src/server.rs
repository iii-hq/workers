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

use std::net::SocketAddr;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;

use anyhow::Context;
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
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
/// live `axum::serve` task; [`rebind`](ServerHandle::rebind) binds a new
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

/// Spawn an `axum::serve` task on `listener`, returning its graceful-shutdown
/// trigger. The `JoinHandle` is intentionally detached: the task ends when its
/// shutdown trigger fires and in-flight connections drain.
fn spawn_server(listener: TcpListener, state: ProxyState) -> watch::Sender<bool> {
    let (tx, mut rx) = watch::channel(false);
    let app = router(state).into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move {
        let shutdown = async move {
            while rx.changed().await.is_ok() {
                if *rx.borrow() {
                    break;
                }
            }
        };
        if let Err(e) = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await
        {
            tracing::error!(error = %e, "rbac-proxy server task exited with error");
        }
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkerConfig;

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
