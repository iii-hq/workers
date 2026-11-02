//! KAN-56: iii-sdk 0.25 carries JSON envelopes above the old 16 MiB cap
//! (0.24.6 rejected them). Drives the public SDK over a loopback WebSocket
//! engine (no provider, no external service): an envelope of the observed
//! 16_863_832 bytes must round-trip in both directions, and the 64 MiB
//! message limit is inclusive.
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, InitOptions, RegisterFunction};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

/// Serialized envelope size seen in production, above the 16 MiB frame cap.
const OBSERVED_ENVELOPE: usize = 16_863_832;
const OLD_LIMIT: usize = 16 * 1024 * 1024;
const MESSAGE_LIMIT: usize = 64 * 1024 * 1024;
const WAIT: Duration = Duration::from_secs(60);

/// What the engine saw: size of the text message, and its parsed JSON.
struct Seen {
    len: usize,
    value: Value,
}

struct Engine {
    url: String,
    seen: mpsc::UnboundedReceiver<Seen>,
    push: mpsc::UnboundedSender<String>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// A reply envelope whose serialized size is exactly `size` bytes.
fn result_envelope(invocation_id: &Value, size: usize) -> String {
    let build = |pad: usize| {
        json!({"type": "invocationresult", "invocation_id": invocation_id,
            "function_id": "test::big", "result": "x".repeat(pad)})
        .to_string()
    };
    let envelope = build(size - build(0).len());
    assert_eq!(envelope.len(), size);
    envelope
}

impl Engine {
    /// Answers `test::big` with a `reply_size` envelope when the request
    /// carries a large string, `"ok"` otherwise (calibration).
    async fn start(reply_size: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (seen_tx, seen) = mpsc::unbounded_channel();
        let (push, mut pushed) = mpsc::unbounded_channel::<String>();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let config = WebSocketConfig::default()
                .max_message_size(Some(2 * MESSAGE_LIMIT))
                .max_frame_size(Some(2 * MESSAGE_LIMIT));
            let mut socket = tokio_tungstenite::accept_async_with_config(tcp, Some(config))
                .await
                .unwrap();
            loop {
                let frame = tokio::select! {
                    Some(text) = pushed.recv() => {
                        socket.send(Message::Text(text.into())).await.unwrap();
                        continue;
                    }
                    frame = socket.next() => match frame {
                        Some(Ok(frame)) => frame,
                        _ => break,
                    },
                };
                let Message::Text(text) = frame else { continue };
                let len = text.len();
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                drop(text);
                let reply = (value["type"] == "invokefunction"
                    && value["function_id"] == "test::big"
                    && !value["invocation_id"].is_null())
                .then(|| {
                    if reply_size > 0 && value["data"].as_str().is_some_and(|data| data.len() > 100)
                    {
                        result_envelope(&value["invocation_id"], reply_size)
                    } else {
                        json!({"type": "invocationresult", "result": "ok",
                            "invocation_id": value["invocation_id"],
                            "function_id": "test::big"})
                        .to_string()
                    }
                });
                let _ = seen_tx.send(Seen { len, value });
                if let Some(reply) = reply {
                    if socket.send(Message::Text(reply.into())).await.is_err() {
                        break;
                    }
                }
            }
        });
        Self {
            url,
            seen,
            push,
            server,
        }
    }

    async fn next_of(&mut self, kind: &str) -> Seen {
        tokio::time::timeout(WAIT, async {
            loop {
                let seen = self.seen.recv().await.expect("engine connection open");
                // `test::big` only: the SDK's own housekeeping calls are not ours.
                let ours = kind != "invokefunction" || seen.value["function_id"] == "test::big";
                if seen.value["type"] == kind && ours {
                    return seen;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("engine never saw `{kind}`"))
    }
}

fn request(payload: Value) -> TriggerRequest {
    TriggerRequest {
        function_id: "test::big".into(),
        payload,
        action: None,
        timeout_ms: Some(60_000),
    }
}

/// Bytes the SDK's `invokefunction` envelope adds around the payload string.
async fn request_overhead(engine: &mut Engine, iii: &iii_sdk::IIIClient) -> usize {
    iii.trigger(request(json!(""))).await.unwrap();
    engine.next_of("invokefunction").await.len
}

#[tokio::test(flavor = "multi_thread")]
async fn observed_envelope_round_trips_as_request_and_response() {
    let mut engine = Engine::start(OBSERVED_ENVELOPE).await;
    let iii = register_worker(&engine.url, InitOptions::default());
    let overhead = request_overhead(&mut engine, &iii).await;

    let payload = "x".repeat(OBSERVED_ENVELOPE - overhead);
    let result = iii.trigger(request(json!(payload))).await.unwrap();

    // Request: the engine received the whole envelope, above the old cap.
    let seen = engine.next_of("invokefunction").await;
    assert_eq!(seen.len, OBSERVED_ENVELOPE);
    assert!(seen.len > OLD_LIMIT);
    assert_eq!(seen.value["data"].as_str().unwrap().len(), payload.len());
    // Response: the SDK accepted a reply envelope of the same size.
    // The reply envelope is OBSERVED_ENVELOPE bytes; its framing is < 200.
    let reply = result.as_str().unwrap().len();
    assert!(reply > OLD_LIMIT && reply < OBSERVED_ENVELOPE, "{reply}");
    assert!(OBSERVED_ENVELOPE - reply < 200, "{reply}");
    iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn observed_envelope_is_accepted_as_inbound_request_and_returned() {
    let mut engine = Engine::start(0).await;
    let iii = register_worker(&engine.url, InitOptions::default());
    iii.register_function(
        "test::echo",
        RegisterFunction::new_async(|data: Value| async move { Ok(data) }),
    );
    engine.next_of("registerfunction").await;

    let build = |pad: usize| {
        json!({"type": "invokefunction", "invocation_id": "00000000-0000-4000-8000-000000000001",
            "function_id": "test::echo", "data": "x".repeat(pad)})
        .to_string()
    };
    let envelope = build(OBSERVED_ENVELOPE - build(0).len());
    assert_eq!(envelope.len(), OBSERVED_ENVELOPE);
    engine.push.send(envelope).unwrap();

    let seen = engine.next_of("invocationresult").await;
    assert!(
        seen.len > OLD_LIMIT,
        "result envelope was {} bytes",
        seen.len
    );
    assert!(seen.value["error"].is_null(), "{}", seen.value["error"]);
    assert_eq!(
        seen.value["result"].as_str().unwrap().len(),
        OBSERVED_ENVELOPE - build(0).len()
    );
    iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn sixty_four_mib_message_limit_is_inclusive() {
    let mut engine = Engine::start(0).await;
    let iii = register_worker(&engine.url, InitOptions::default());
    let overhead = request_overhead(&mut engine, &iii).await;

    let at_limit = "x".repeat(MESSAGE_LIMIT - overhead);
    iii.trigger(request(json!(at_limit))).await.unwrap();
    assert_eq!(engine.next_of("invokefunction").await.len, MESSAGE_LIMIT);

    let over = "x".repeat(MESSAGE_LIMIT - overhead + 1);
    let error = iii.trigger(request(json!(over))).await.unwrap_err();
    assert!(
        format!("{error:?}").contains("payload_too_large"),
        "{error:?}"
    );
    iii.shutdown();
}
