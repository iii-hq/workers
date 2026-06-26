//! The data plane: one outbound engine WebSocket per inbound connection, with
//! the RBAC [`Interceptor`](crate::interceptor) spliced into each pump half.
//!
//! This is the [`console`](../../console) `src/proxy.rs` pattern — `tokio::
//! select!` on the two pump halves, 1:1 frame conversion between
//! [`axum::extract::ws::Message`] and [`tokio_tungstenite::tungstenite::
//! Message`] — except: the upgrade is **authenticated first** (no upstream is
//! opened on a rejected connection), and each Text frame is routed through the
//! interceptor instead of being shuttled blindly.
//!
//! A single writer task owns each socket's sink and drains an `mpsc`, so the
//! two pump directions (and synthesized replies) never contend for a sink.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use axum::extract::ws::{CloseFrame as AxumCloseFrame, Message as AxumMessage, WebSocket};
use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::http::{HeaderMap, Uri};
use axum::response::Response;
use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::CloseFrame as TungCloseFrame;
use tokio_tungstenite::tungstenite::Message as TungMessage;

use crate::interceptor::{DownstreamAction, Interceptor, UpstreamAction};
use crate::rbac;
use crate::server::ProxyState;

/// Axum handler for the worker-protocol route (`GET /`). Captures the upgrade
/// request's headers, query, and peer address (for the auth function) before
/// the upgrade, then hands the socket to [`handle_connection`].
pub async fn ws_upgrade(
    State(state): State<ProxyState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    uri: Uri,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| handle_connection(socket, state, peer, uri, headers))
}

/// Decrement the live-connection gauge when a connection ends.
struct ActiveGuard(Arc<AtomicU32>);
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn handle_connection(
    client: WebSocket,
    state: ProxyState,
    peer: SocketAddr,
    uri: Uri,
    headers: HeaderMap,
) {
    // Snapshot the config at the upgrade. The connection keeps these
    // boundaries for its lifetime; a hot reload affects only later upgrades.
    let cfg = state.config.read().await.clone();
    let ip = peer.ip().to_string();

    // 1. Authenticate (control connection). Reject → error frame + Close,
    //    upstream is never opened.
    let session = match rbac::resolve_session(
        &state.iii,
        Some(&cfg.rbac),
        headers_to_map(&headers),
        query_to_multi_map(uri.query()),
        ip,
    )
    .await
    {
        Ok(s) => Arc::new(s),
        Err(rej) => {
            reject(client, &rej.code, &rej.message).await;
            return;
        }
    };

    // 2. Dial the upstream engine listener.
    let (engine, _resp) = match tokio_tungstenite::connect_async(cfg.engine_url.as_str()).await {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!(
                error = %e,
                engine_url = %crate::redact_url(&cfg.engine_url),
                "failed to dial upstream engine; closing downstream"
            );
            let mut client = client;
            let _ = client
                .send(AxumMessage::Close(Some(AxumCloseFrame {
                    code: 1011, // internal error
                    reason: "engine WS dial failed".into(),
                })))
                .await;
            return;
        }
    };

    state.active.fetch_add(1, Ordering::Relaxed);
    let _guard = ActiveGuard(state.active.clone());

    // Out-of-band reply channel for spawned middleware results (see the
    // Interceptor's `reply_tx` — spawning avoids a head-of-line deadlock).
    let (reply_tx, mut reply_rx) = mpsc::channel::<String>(128);

    let interceptor = Arc::new(Interceptor::new(
        state.iii.clone(),
        cfg.clone(),
        state.catalog.clone(),
        session,
        reply_tx,
    ));

    let (mut client_sink, mut client_rx) = client.split();
    let (mut engine_sink, mut engine_rx) = engine.split();

    // One writer per socket; both pump directions (and the reply forwarder)
    // feed the client writer.
    let (client_out, mut client_out_rx) = mpsc::channel::<AxumMessage>(128);
    let (engine_out, mut engine_out_rx) = mpsc::channel::<TungMessage>(128);

    let client_writer = tokio::spawn(async move {
        while let Some(m) = client_out_rx.recv().await {
            if client_sink.send(m).await.is_err() {
                break;
            }
        }
        let _ = client_sink.close().await;
    });

    // Forward spawned middleware replies to the client writer.
    let reply_client_out = client_out.clone();
    let reply_forwarder = tokio::spawn(async move {
        while let Some(text) = reply_rx.recv().await {
            if reply_client_out
                .send(AxumMessage::Text(text))
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let engine_writer = tokio::spawn(async move {
        while let Some(m) = engine_out_rx.recv().await {
            if engine_sink.send(m).await.is_err() {
                break;
            }
        }
        let _ = engine_sink.close().await;
    });

    // Direction A: downstream worker → engine (intercept Text frames).
    let a_int = interceptor.clone();
    let a_client_out = client_out.clone();
    let a_engine_out = engine_out.clone();
    let client_to_engine = async move {
        while let Some(msg) = client_rx.next().await {
            let Ok(msg) = msg else { break };
            let is_close = matches!(msg, AxumMessage::Close(_));
            match msg {
                AxumMessage::Text(t) => match a_int.handle_downstream(t.as_str()).await {
                    DownstreamAction::Forward(out) => {
                        if a_engine_out.send(TungMessage::Text(out)).await.is_err() {
                            break;
                        }
                    }
                    DownstreamAction::ReplyToClient(reply) => {
                        if a_client_out.send(AxumMessage::Text(reply)).await.is_err() {
                            break;
                        }
                    }
                    DownstreamAction::Drop => {}
                },
                other => {
                    if let Some(out) = axum_to_tungstenite(other) {
                        if a_engine_out.send(out).await.is_err() {
                            break;
                        }
                    }
                }
            }
            if is_close {
                break;
            }
        }
    };

    // Direction B: engine → downstream worker (rewrite Text frames).
    let b_int = interceptor.clone();
    let b_client_out = client_out.clone();
    let engine_to_client = async move {
        while let Some(msg) = engine_rx.next().await {
            let Ok(msg) = msg else { break };
            let is_close = matches!(msg, TungMessage::Close(_));
            match msg {
                TungMessage::Text(t) => {
                    let UpstreamAction::Forward(out) = b_int.handle_upstream(t.as_str()).await;
                    if b_client_out.send(AxumMessage::Text(out)).await.is_err() {
                        break;
                    }
                }
                other => {
                    if let Some(out) = tungstenite_to_axum(other) {
                        if b_client_out.send(out).await.is_err() {
                            break;
                        }
                    }
                }
            }
            if is_close {
                break;
            }
        }
    };

    tokio::select! {
        _ = client_to_engine => {}
        _ = engine_to_client => {}
    }

    // One side closed: tear the connection down promptly. Aborting the writers
    // drops their socket sinks (closing both sockets); the engine then tears
    // down this connection's functions/triggers via its existing per-connection
    // cleanup. Any in-flight spawned middleware task finishes on its own — its
    // reply send fails harmlessly once the forwarder is gone.
    client_writer.abort();
    engine_writer.abort();
    reply_forwarder.abort();
    // _guard (ActiveGuard) drops here → live-connection gauge decremented.
}

/// Send the engine's out-of-band rejection frame, then Close — the exact shape
/// and ordering the engine uses on a failed RBAC handshake (engine/mod.rs:
/// 1446-1455), which SDK clients special-case. The upstream is never opened.
async fn reject(client: WebSocket, code: &str, message: &str) {
    let mut client = client;
    let frame = serde_json::json!({
        "type": "error",
        "error": { "code": code, "message": message }
    });
    let _ = client.send(AxumMessage::Text(frame.to_string())).await;
    let _ = client.send(AxumMessage::Close(None)).await;
}

/// Build the `AuthInput.headers` map (`Record<string, string>`). Header names
/// are already lowercased by axum; non-UTF-8 values are skipped.
fn headers_to_map(headers: &HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_string(), v.to_string()))
        })
        .collect()
}

/// Build the `AuthInput.query_params` map (`Record<string, string[]>`),
/// preserving repeated keys, with values URL-decoded.
fn query_to_multi_map(query: Option<&str>) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(q) = query {
        for (k, v) in url::form_urlencoded::parse(q.as_bytes()) {
            map.entry(k.into_owned()).or_default().push(v.into_owned());
        }
    }
    map
}

/// Convert an axum `Message` into a tungstenite `Message` (verbatim from
/// `console`). Returns `None` when the variant has no useful equivalent.
pub(crate) fn axum_to_tungstenite(msg: AxumMessage) -> Option<TungMessage> {
    Some(match msg {
        AxumMessage::Text(t) => TungMessage::Text(t),
        AxumMessage::Binary(b) => TungMessage::Binary(b),
        AxumMessage::Ping(p) => TungMessage::Ping(p),
        AxumMessage::Pong(p) => TungMessage::Pong(p),
        AxumMessage::Close(frame) => TungMessage::Close(frame.map(|f| TungCloseFrame {
            code: f.code.into(),
            reason: f.reason.into_owned().into(),
        })),
    })
}

/// Convert a tungstenite `Message` into an axum `Message` (verbatim from
/// `console`). Returns `None` for raw `Frame`.
pub(crate) fn tungstenite_to_axum(msg: TungMessage) -> Option<AxumMessage> {
    Some(match msg {
        TungMessage::Text(t) => AxumMessage::Text(t),
        TungMessage::Binary(b) => AxumMessage::Binary(b),
        TungMessage::Ping(p) => AxumMessage::Ping(p),
        TungMessage::Pong(p) => AxumMessage::Pong(p),
        TungMessage::Close(frame) => AxumMessage::Close(frame.map(|f| AxumCloseFrame {
            code: u16::from(f.code),
            reason: f.reason.into_owned().into(),
        })),
        TungMessage::Frame(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_multi_map_preserves_repeated_keys_and_decodes() {
        let m = query_to_multi_map(Some("a=1&a=2&b=hello%20world"));
        assert_eq!(m.get("a"), Some(&vec!["1".to_string(), "2".to_string()]));
        assert_eq!(m.get("b"), Some(&vec!["hello world".to_string()]));
    }

    #[test]
    fn query_multi_map_empty() {
        assert!(query_to_multi_map(None).is_empty());
        assert!(query_to_multi_map(Some("")).is_empty());
    }

    #[test]
    fn text_roundtrips_both_directions() {
        let original = AxumMessage::Text("hello".into());
        let tung = axum_to_tungstenite(original).unwrap();
        assert!(matches!(&tung, TungMessage::Text(s) if s.as_str() == "hello"));
        let back = tungstenite_to_axum(tung).unwrap();
        assert!(matches!(&back, AxumMessage::Text(s) if s.as_str() == "hello"));
    }

    #[test]
    fn close_preserves_code_and_reason() {
        let original = AxumMessage::Close(Some(AxumCloseFrame {
            code: 1001,
            reason: "going away".into(),
        }));
        let tung = axum_to_tungstenite(original).unwrap();
        let frame = match tung {
            TungMessage::Close(f) => f.unwrap(),
            _ => panic!("expected close"),
        };
        assert_eq!(u16::from(frame.code), 1001);
        assert_eq!(&*frame.reason, "going away");
    }
}
