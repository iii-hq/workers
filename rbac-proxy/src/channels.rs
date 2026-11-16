//! Relays: the channel bridge and the authenticated relay of other exposed
//! endpoints (`/otel`) share [`relay`].
//!
//! Channel bridge: `GET /ws/channels/{channel_id}` on the proxy's RBAC port,
//! relayed verbatim to `{engine_url}/ws/channels/{channel_id}` with the same
//! `?key=&dir=` query string.
//!
//! A `StreamChannelRef` carries **no host**, so the SDK builds the channel URL
//! from the address the worker connected to (the proxy). The proxy therefore
//! mounts the route and bridges it — no ref rewriting is needed or possible
//! (spec *Channel bridge*, Pattern A). This is a **dumb relay**: frames pass
//! 1:1 in both directions; the engine independently validates the `access_key`
//! capability token (404 before upgrade on a bad key), so the proxy does not
//! re-authenticate the channel socket.

use std::sync::atomic::Ordering;

use axum::extract::ws::{CloseFrame, Message as AxumMessage, WebSocket};
use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use tokio_tungstenite::tungstenite::Message as TungMessage;

use crate::endpoints::{endpoint_exposed, upstream_url};
use crate::proxy::{axum_to_tungstenite, tungstenite_to_axum, ActiveGuard};
use crate::server::ProxyState;

/// Relay a channel socket to the engine's `/ws/channels/{channel_id}` with
/// the same query string (the `access_key` the engine checks).
pub async fn channel(
    client: WebSocket,
    state: ProxyState,
    engine_url: String,
    path: String,
    query: Option<String>,
) {
    let target = upstream_url(&engine_url, &path, query.as_deref());
    relay(client, state, path, target, "engine channel dial failed").await
}

/// How long a relay waits to close both sockets once it ends. A client that
/// stops reading can leave the Close send pending forever; past this the
/// relay returns anyway, so its `active_connections` slot and sockets drop.
const CLEANUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Run `cleanup` for at most `limit`; false when it did not finish in time.
async fn within(
    limit: std::time::Duration,
    cleanup: impl std::future::Future<Output = ()>,
) -> bool {
    tokio::time::timeout(limit, cleanup).await.is_ok()
}

/// Resolves once the configuration stops exposing `path`: at once when it
/// already does not (a reload landed while the engine dial was pending),
/// otherwise on the reload that withdraws it. It subscribes before the first
/// check, so a reload between the two still wakes it.
async fn unexposed(state: &ProxyState, path: &str) {
    let mut changes = crate::configuration::config_changes();
    loop {
        let cfg = state.config.read().await.clone();
        if !endpoint_exposed(&cfg.rbac.endpoints(), path) {
            return;
        }
        if changes.changed().await.is_err() {
            return std::future::pending().await;
        }
    }
}

/// Dial `target` on the engine and pass frames 1:1 both ways until either
/// side closes, or a reload stops exposing `path` (close 1008). The socket
/// counts in `active_connections` while it lasts. A failed dial closes the
/// client with 1011 and `dial_failed` as the reason.
pub async fn relay(
    client: WebSocket,
    state: ProxyState,
    path: String,
    target: String,
    dial_failed: &'static str,
) {
    let (engine, _resp) =
        match tokio_tungstenite::connect_async_with_config(&target, None, true).await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    target = %crate::redact_url(&target),
                    "failed to dial the engine endpoint; closing downstream (1011)"
                );
                let mut client = client;
                let _ = client
                    .send(AxumMessage::Close(Some(CloseFrame {
                        code: 1011,
                        reason: dial_failed.into(),
                    })))
                    .await;
                return;
            }
        };
    state.active.fetch_add(1, Ordering::Relaxed);
    let _guard = ActiveGuard(state.active.clone());

    let (mut client_tx, mut client_rx) = client.split();
    let (mut engine_tx, mut engine_rx) = engine.split();

    let client_to_engine = async {
        while let Some(Ok(msg)) = client_rx.next().await {
            let is_close = matches!(msg, AxumMessage::Close(_));
            if let Some(out) = axum_to_tungstenite(msg) {
                if engine_tx.send(out).await.is_err() {
                    break;
                }
            }
            if is_close {
                break;
            }
        }
        false
    };

    let engine_to_client = async {
        while let Some(Ok(msg)) = engine_rx.next().await {
            let is_close = matches!(msg, TungMessage::Close(_));
            if let Some(out) = tungstenite_to_axum(msg) {
                if client_tx.send(out).await.is_err() {
                    break;
                }
            }
            if is_close {
                break;
            }
        }
        false
    };

    let withdrawn = tokio::select! {
        w = client_to_engine => w,
        w = engine_to_client => w,
        _ = unexposed(&state, &path) => true,
    };
    if withdrawn {
        tracing::info!(path = %path, "endpoint no longer exposed; closing its relay");
    }
    let cleanup = async {
        if withdrawn {
            let _ = client_tx
                .send(AxumMessage::Close(Some(CloseFrame {
                    code: 1008,
                    reason: "endpoint no longer exposed".into(),
                })))
                .await;
        }
        let _ = client_tx.close().await;
        let _ = engine_tx.close().await;
    };
    if !within(CLEANUP_TIMEOUT, cleanup).await {
        tracing::debug!(path = %path, "relay cleanup timed out; dropping the sockets");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkerConfig;
    use crate::endpoints::EndpointFilter;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::RwLock;

    fn config(endpoints: &[&str]) -> WorkerConfig {
        let mut cfg = WorkerConfig::default();
        cfg.rbac.expose_endpoints = Some(
            endpoints
                .iter()
                .map(|p| EndpointFilter::new(p).expect("a valid rule"))
                .collect(),
        );
        cfg
    }

    fn state(endpoints: &[&str]) -> ProxyState {
        let iii = Arc::new(iii_sdk::register_worker(
            "ws://127.0.0.1:1",
            iii_sdk::InitOptions::default(),
        ));
        ProxyState::new(Arc::new(RwLock::new(Arc::new(config(endpoints)))), iii)
    }

    #[tokio::test]
    async fn a_path_withdrawn_before_the_relay_starts_closes_at_once() {
        // The reload landed while the engine dial was pending: no later
        // reload is coming, so waiting for one would relay a withdrawn path.
        let state = state(&["/"]);
        tokio::time::timeout(Duration::from_secs(2), unexposed(&state, "/ws/channels/c1"))
            .await
            .expect("an unexposed path resolves without another reload");
    }

    #[tokio::test]
    async fn an_exposed_path_waits_for_the_reload_that_withdraws_it() {
        let state = state(&["/", "/ws/channels/*"]);
        let wait = unexposed(&state, "/ws/channels/c1");
        tokio::pin!(wait);
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut wait)
                .await
                .is_err(),
            "still exposed: the relay stays open"
        );
        crate::configuration::apply_config(&state.config, config(&["/"])).await;
        tokio::time::timeout(Duration::from_secs(2), &mut wait)
            .await
            .expect("the withdrawing reload closes it");
    }

    #[tokio::test]
    async fn a_stalled_cleanup_gives_up_at_its_limit() {
        // A client that stops reading never completes the Close send; the
        // relay must still return so its guard and sockets drop.
        let started = std::time::Instant::now();
        assert!(!within(Duration::from_millis(50), std::future::pending::<()>()).await);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(within(Duration::from_millis(50), async {}).await);
    }
}
