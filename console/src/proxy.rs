//! Transparent WebSocket proxy at `/ws`.
//!
//! For every incoming `/ws` upgrade we open a fresh outbound WebSocket
//! to the iii engine (`engine_url`) and pump frames between the two
//! sockets until either side closes. Frame types are translated 1:1
//! between [`axum::extract::ws::Message`] and
//! [`tokio_tungstenite::tungstenite::Message`].
//!
//! The proxy is intentionally dumb: no buffering, no rewriting, no auth.
//! The engine WebSocket and the iii-browser-sdk client on the page do
//! all the framing — this module just shuttles bytes.

use std::sync::Arc;

use axum::extract::ws::{CloseFrame as AxumCloseFrame, Message as AxumMessage, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::Response;
use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use tokio_tungstenite::tungstenite::protocol::CloseFrame as TungCloseFrame;
use tokio_tungstenite::tungstenite::Message as TungMessage;

/// Axum handler. Splits the upgraded socket into a sender + receiver
/// and spawns a single `handle_ws` future for the lifetime of the
/// connection.
pub async fn ws_proxy(ws: WebSocketUpgrade, State(engine_url): State<Arc<String>>) -> Response {
    ws.on_upgrade(move |socket| handle_ws(socket, engine_url))
}

async fn handle_ws(client: WebSocket, engine_url: Arc<String>) {
    let (engine, _resp) = match tokio_tungstenite::connect_async(engine_url.as_str()).await {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!(
                error = %e,
                engine_url = %engine_url,
                "failed to dial iii engine WebSocket; closing browser WS"
            );
            // Best-effort close; ignore the result. axum will drop the
            // socket once `client` goes out of scope anyway.
            let mut client = client;
            let _ = client
                .send(AxumMessage::Close(Some(AxumCloseFrame {
                    code: 1011, // 1011 = "internal error"
                    reason: "engine WS dial failed".into(),
                })))
                .await;
            return;
        }
    };

    let (mut client_tx, mut client_rx) = client.split();
    let (mut engine_tx, mut engine_rx) = engine.split();

    let client_to_engine = async {
        while let Some(msg) = client_rx.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(e) => {
                    tracing::debug!(error = %e, "browser -> engine: client read error");
                    break;
                }
            };
            let is_close = matches!(msg, AxumMessage::Close(_));
            if let Some(out) = axum_to_tungstenite(msg) {
                if let Err(e) = engine_tx.send(out).await {
                    tracing::debug!(error = %e, "browser -> engine: engine send error");
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
        while let Some(msg) = engine_rx.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(e) => {
                    tracing::debug!(error = %e, "engine -> browser: engine read error");
                    break;
                }
            };
            let is_close = matches!(msg, TungMessage::Close(_));
            if let Some(out) = tungstenite_to_axum(msg) {
                if let Err(e) = client_tx.send(out).await {
                    tracing::debug!(error = %e, "engine -> browser: client send error");
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

/// Convert an axum `Message` into a tungstenite `Message`. Returns
/// `None` when the variant has no useful tungstenite equivalent.
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

/// Convert a tungstenite `Message` into an axum `Message`. Returns
/// `None` for `Frame` (raw frames are an internal tungstenite escape
/// hatch the proxy never produces or forwards).
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
    fn text_roundtrips_both_directions() {
        let original = AxumMessage::Text("hello".into());
        let tung = axum_to_tungstenite(original.clone()).unwrap();
        assert!(matches!(&tung, TungMessage::Text(s) if s == "hello"));
        let back = tungstenite_to_axum(tung).unwrap();
        assert!(matches!(&back, AxumMessage::Text(s) if s == "hello"));
    }

    #[test]
    fn binary_roundtrips_both_directions() {
        let original = AxumMessage::Binary(vec![1, 2, 3, 4]);
        let tung = axum_to_tungstenite(original.clone()).unwrap();
        assert!(matches!(&tung, TungMessage::Binary(b) if b == &vec![1, 2, 3, 4]));
        let back = tungstenite_to_axum(tung).unwrap();
        assert!(matches!(&back, AxumMessage::Binary(b) if b == &vec![1, 2, 3, 4]));
    }

    #[test]
    fn ping_pong_pass_through() {
        let p = axum_to_tungstenite(AxumMessage::Ping(vec![9])).unwrap();
        assert!(matches!(&p, TungMessage::Ping(b) if b == &vec![9]));
        let p = axum_to_tungstenite(AxumMessage::Pong(vec![10])).unwrap();
        assert!(matches!(&p, TungMessage::Pong(b) if b == &vec![10]));
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
        assert_eq!(frame.reason, "going away");

        let back = tungstenite_to_axum(TungMessage::Close(Some(frame))).unwrap();
        match back {
            AxumMessage::Close(Some(f)) => {
                assert_eq!(f.code, 1001);
                assert_eq!(f.reason, "going away");
            }
            _ => panic!("expected close"),
        }
    }

    #[test]
    fn raw_frame_is_dropped() {
        // Raw frames are an internal tungstenite construct; we don't
        // forward them. Use a real Frame ctor — `default()` panics.
        let frame = tokio_tungstenite::tungstenite::protocol::frame::Frame::ping(vec![]);
        assert!(tungstenite_to_axum(TungMessage::Frame(frame)).is_none());
    }
}
