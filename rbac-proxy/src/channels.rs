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

use axum::extract::ws::{Message as AxumMessage, WebSocket};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::http::Uri;
use axum::response::Response;
use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use tokio_tungstenite::tungstenite::Message as TungMessage;

use crate::proxy::{axum_to_tungstenite, tungstenite_to_axum};
use crate::server::ProxyState;

/// Axum handler for `GET /ws/channels/{channel_id}`.
pub async fn channel_bridge(
    State(state): State<ProxyState>,
    Path(channel_id): Path<String>,
    uri: Uri,
    ws: WebSocketUpgrade,
) -> Response {
    // Read the upstream base live so a hot `engine_url` change reaches new
    // channel sockets (consistent with the data plane).
    let engine_url = state.config.read().await.engine_url.clone();
    let query = uri.query().map(str::to_string);
    ws.on_upgrade(move |socket| bridge(socket, engine_url, channel_id, query))
}

async fn bridge(client: WebSocket, engine_url: String, channel_id: String, query: Option<String>) {
    let base = engine_url.trim_end_matches('/');
    let mut target = format!("{base}/ws/channels/{channel_id}");
    if let Some(q) = query.filter(|q| !q.is_empty()) {
        target.push('?');
        target.push_str(&q);
    }

    let (engine, _resp) = match tokio_tungstenite::connect_async(&target).await {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!(
                error = %e,
                channel_id = %channel_id,
                "failed to dial engine channel socket; closing downstream (1011)"
            );
            let mut client = client;
            let _ = client
                .send(AxumMessage::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011,
                    reason: "engine channel dial failed".into(),
                })))
                .await;
            return;
        }
    };

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
        let _ = engine_tx.close().await;
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
        let _ = client_tx.close().await;
    };

    tokio::select! {
        _ = client_to_engine => {}
        _ = engine_to_client => {}
    }
}
