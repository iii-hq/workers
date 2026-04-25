//! Integration tests for `iii-a2a-client`.
//!
//! Two layers of coverage:
//!
//! 1. **Mock-A2A round-trip (engine-free):** boot an `axum` server that
//!    serves `/.well-known/agent-card.json` + `POST /a2a`, then exercise
//!    `Session::send_message` and `Session::stream_message` directly. These
//!    tests run on every `cargo test`.
//!
//! 2. **Live-engine end-to-end:** the `register_all` + `iii.trigger` path
//!    needs a real engine on `ws://127.0.0.1:49134`. Those tests are gated
//!    with `#[ignore]`; run them with `cargo test -- --ignored` once an
//!    engine is up.
//!
//! Mock A2A server contract (mirrors `iii-a2a`):
//!
//! - `GET /.well-known/agent-card.json` → AgentCard with two skills:
//!   `greet` and `slow_compute`.
//! - `POST /a2a` with `message/send` → returns a Completed Task whose first
//!   artifact part carries `{"hello": "world"}` (or echoes the payload back
//!   when present).
//! - `POST /a2a` with `message/stream` → SSE; emits one
//!   `TaskStatusUpdateEvent` then closes.

use std::net::SocketAddr;
use std::time::Duration;

use axum::{
    extract::State,
    http::HeaderMap,
    response::{sse::Event, IntoResponse, Sse},
    routing::{get, post},
    Json, Router,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::oneshot;

// -------- Mock A2A server --------

#[derive(Clone)]
struct MockState {
    streaming: bool,
}

fn agent_card(streaming: bool) -> Value {
    json!({
        "name": "mock",
        "description": "mock A2A agent for iii-a2a-client tests",
        "version": "0.0.1",
        "supportedInterfaces": [{
            "url": "http://localhost/a2a",
            "protocolBinding": "JSONRPC",
            "protocolVersion": "0.3"
        }],
        "provider": { "organization": "Test", "url": "" },
        "capabilities": {
            "streaming": streaming,
            "pushNotifications": false,
            "stateTransitionHistory": false
        },
        "defaultInputModes": ["application/json"],
        "defaultOutputModes": ["application/json"],
        "skills": [
            { "id": "greet", "name": "Greet", "description": "Say hello", "tags": ["demo"] },
            { "id": "slow_compute", "name": "Slow", "description": "Heavier work", "tags": ["demo"] }
        ]
    })
}

async fn handle_card(State(state): State<MockState>) -> impl IntoResponse {
    Json(agent_card(state.streaming))
}

async fn handle_a2a(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> axum::response::Response {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");

    if method == "message/stream" {
        if !state.streaming {
            let body = json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32004, "message": "Streaming not supported" }
            });
            return (axum::http::StatusCode::OK, Json(body)).into_response();
        }
        // Honour the SSE accept hint (debugging aid only).
        let _ = headers.get("accept");
        let stream = futures_util::stream::iter(vec![Ok::<_, std::convert::Infallible>(
            Event::default().data(
                serde_json::to_string(&json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "taskId": "stream-task-1",
                        "status": { "state": "completed" },
                        "final": true
                    }
                }))
                .unwrap(),
            ),
        )]);
        return Sse::new(stream).into_response();
    }

    // message/send (default)
    let echo_payload: Value = req
        .get("params")
        .and_then(|p| p.get("message"))
        .and_then(|m| m.get("parts"))
        .and_then(|ps| ps.as_array())
        .and_then(|arr| arr.iter().find_map(|p| p.get("data")))
        .and_then(|d| d.get("payload"))
        .cloned()
        .unwrap_or(json!({}));

    let result_obj =
        if echo_payload.is_object() && echo_payload.as_object().is_some_and(|m| !m.is_empty()) {
            echo_payload
        } else {
            json!({ "hello": "world" })
        };
    let result_text = serde_json::to_string(&result_obj).unwrap();

    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "task": {
                "id": "task-1",
                "status": { "state": "completed" },
                "artifacts": [{
                    "artifactId": "a1",
                    "parts": [{
                        "text": result_text,
                        "mediaType": "application/json"
                    }],
                    "name": "result"
                }]
            }
        }
    });
    (axum::http::StatusCode::OK, Json(body)).into_response()
}

async fn boot_mock(streaming: bool) -> (String, oneshot::Sender<()>) {
    let app = Router::new()
        .route("/.well-known/agent-card.json", get(handle_card))
        .route("/a2a", post(handle_a2a))
        .with_state(MockState { streaming });

    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async move {
            let _ = rx.await;
        });
        let _ = server.await;
    });

    // Give the server a moment to come up.
    tokio::time::sleep(Duration::from_millis(50)).await;
    (format!("http://{addr}"), tx)
}

// -------- Engine-free tests --------

#[tokio::test]
async fn session_connect_parses_card() {
    let (base, _shutdown) = boot_mock(false).await;
    let session = iii_a2a_client::session::Session::connect(&base)
        .await
        .expect("connect to mock");

    assert_eq!(session.name, "test_mock");
    assert_eq!(session.base_url, base);
    let card = session.card.read().await;
    assert_eq!(card.skills.len(), 2);
    assert!(card.skills.iter().any(|s| s.id == "greet"));
}

#[tokio::test]
async fn session_send_message_returns_completed_task() {
    let (base, _shutdown) = boot_mock(false).await;
    let session = iii_a2a_client::session::Session::connect(&base)
        .await
        .expect("connect to mock");
    let task = session
        .send_message("greet", json!({}))
        .await
        .expect("send_message ok");

    use iii_a2a_client::types::TaskState;
    assert_eq!(task.status.state, TaskState::Completed);
    let part = task
        .artifacts
        .as_ref()
        .and_then(|a| a.first())
        .and_then(|art| art.parts.first())
        .expect("artifact present");
    let text = part.text.as_deref().unwrap_or("");
    let value: Value = serde_json::from_str(text).expect("artifact text is JSON");
    assert_eq!(value, json!({ "hello": "world" }));
}

#[tokio::test]
async fn session_stream_message_yields_status_event() {
    let (base, _shutdown) = boot_mock(true).await;
    let session = iii_a2a_client::session::Session::connect(&base)
        .await
        .expect("connect to mock");
    let mut stream = Box::pin(
        session
            .stream_message("slow_compute", json!({}))
            .await
            .expect("stream init ok"),
    );

    let mut saw_status = false;
    while let Some(item) = stream.next().await {
        match item {
            Ok(iii_a2a_client::types::StreamEvent::Status(_)) => {
                saw_status = true;
                break;
            }
            Ok(iii_a2a_client::types::StreamEvent::Task(_)) => {
                // Some servers ship the final task as the closing event;
                // accept that as a signal too.
                saw_status = true;
                break;
            }
            Ok(_) => {}
            Err(e) => panic!("stream error: {e}"),
        }
    }
    assert!(saw_status, "expected at least one status/task event");
}

// -------- Engine-bound tests (require ws://127.0.0.1:49134) --------

#[tokio::test]
#[ignore = "requires a running iii engine on ws://127.0.0.1:49134"]
async fn live_engine_register_and_trigger() {
    use iii_sdk::{register_worker, InitOptions};

    let (base, _shutdown) = boot_mock(false).await;
    let iii = register_worker("ws://127.0.0.1:49134", InitOptions::default());

    let session = iii_a2a_client::session::Session::connect(&base)
        .await
        .expect("connect to mock");
    let session_name = session.name.clone();
    let _map = iii_a2a_client::registration::register_all(&iii, session, "a2a").await;

    // Give the engine a beat to ack the registrations.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let fns = iii.list_functions().await.expect("list_functions");
    let greet_id = format!("a2a.{session_name}::greet");
    let slow_id = format!("a2a.{session_name}::slow_compute");
    assert!(
        fns.iter().any(|f| f.function_id == greet_id),
        "greet not registered: {:?}",
        fns.iter().map(|f| &f.function_id).collect::<Vec<_>>()
    );
    assert!(fns.iter().any(|f| f.function_id == slow_id));

    let result: Value = iii
        .trigger(iii_sdk::TriggerRequest {
            function_id: greet_id,
            payload: json!({}),
            action: None,
            timeout_ms: Some(5000),
        })
        .await
        .expect("trigger greet");
    assert_eq!(result, json!({ "hello": "world" }));

    drop(_shutdown);
}

#[tokio::test]
#[ignore = "requires a running iii engine + streaming-capable mock"]
async fn live_engine_streaming_capable_mock() {
    use iii_sdk::{register_worker, InitOptions};

    let (base, _shutdown) = boot_mock(true).await;
    let _iii = register_worker("ws://127.0.0.1:49134", InitOptions::default());

    let session = iii_a2a_client::session::Session::connect(&base)
        .await
        .expect("connect");
    let mut stream = Box::pin(
        session
            .stream_message("slow_compute", json!({}))
            .await
            .expect("stream"),
    );
    let mut got_event = false;
    while let Some(item) = stream.next().await {
        if item.is_ok() {
            got_event = true;
            break;
        }
    }
    assert!(got_event);
    drop(_shutdown);
}
