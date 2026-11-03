use futures_util::{SinkExt, StreamExt};
use iii_sdk::{register_worker, InitOptions};
use quick_tunnel::{api::*, config::Config, manager::Manager, service};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::mpsc};
use tokio_tungstenite::tungstenite::Message as WsMessage;

#[test]
fn configuration_rejects_nonloopback_and_authority_tricks() {
    for origin in [
        "http://example.com:3112",
        "http://localhost:3112",
        "http://0.0.0.0:3112",
        "http://127.0.0.1:0",
        "http://user:pass@127.0.0.1:3112",
        "file:///tmp/foo",
        "http://127.0.0.1:3112/admin",
        "http://127.0.0.1:3112?x=1",
    ] {
        let config = Config {
            targets: BTreeMap::from([("webhooks".into(), origin.into())]),
            ..Config::default()
        };
        assert!(config.validate().is_err(), "{origin}");
    }
    assert!(Config::default().validate().is_ok());
}

#[test]
fn contracts_default_tunnel_and_reject_arbitrary_origins() {
    let request: AcquireRequest = serde_json::from_value(
        json!({"consumer_id":"github", "expires_at":"2030-01-01T00:00:00Z"}),
    )
    .unwrap();
    assert_eq!(request.tunnel_id, "webhooks");
    assert!(serde_json::from_value::<AcquireRequest>(json!({"consumer_id":"github", "expires_at":"2030-01-01T00:00:00Z", "origin":"http://127.0.0.1:22"})).is_err());
    assert!(serde_json::from_value::<AcquireRequest>(
        json!({"consumer_id":"github", "expires_at":"tomorrow"})
    )
    .is_err());
    assert_eq!(
        serde_json::from_value::<StatusRequest>(json!({}))
            .unwrap()
            .tunnel_id,
        "webhooks"
    );
    let schema = serde_json::to_value(schemars::schema_for!(AcquireRequest)).unwrap();
    assert_eq!(schema["properties"]["expires_at"]["format"], "date-time");
    assert_eq!(schema["additionalProperties"], false);
}

async fn next_matching(
    rx: &mut mpsc::UnboundedReceiver<Value>,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let value = rx.recv().await.unwrap();
            if predicate(&value) {
                break value;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interface_registers_without_cloudflared_and_observers_receive_snapshots() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("ws://{}", listener.local_addr().unwrap());
    let (captured_tx, mut captured) = mpsc::unbounded_channel::<Value>();
    let (wire_tx, mut wire_rx) = mpsc::unbounded_channel::<Value>();
    // An engine with observe.v2: the SDK holds a private function's
    // registration until the engine advertises it.
    wire_tx
        .send(json!({
            "type": "workerregistered",
            "worker_id": "quick-tunnel-test",
            "features": ["observe.v1", "observe.v2"],
        }))
        .unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        loop {
            tokio::select! {
                outbound = wire_rx.recv() => {
                    let Some(outbound) = outbound else { break; };
                    if ws.send(WsMessage::Text(outbound.to_string().into())).await.is_err() { break; }
                }
                inbound = ws.next() => {
                    let text = match inbound {
                        Some(Ok(WsMessage::Text(text))) => text,
                        Some(Ok(WsMessage::Ping(data))) => {
                            if ws.send(WsMessage::Pong(data)).await.is_err() { break; }
                            continue;
                        }
                        Some(Ok(WsMessage::Pong(_))) => continue,
                        _ => break,
                    };
                    let value: Value = serde_json::from_str(&text).unwrap();
                    // Answer the SDK's own calls; a void call has no invocation id.
                    if value["type"] == "invokefunction" && value["invocation_id"].is_string() {
                        let reply = json!({
                            "type": "invocationresult",
                            "invocation_id": value["invocation_id"],
                            "function_id": value["function_id"],
                            "result": { "ok": true },
                        });
                        ws.send(WsMessage::Text(reply.to_string().into())).await.unwrap();
                    }
                    let _ = captured_tx.send(value);
                }
            }
        }
    });
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".test-data");
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::tempdir_in(root).unwrap();
    let manager = Manager::open(Config {
        cloudflared: "/nonexistent/cloudflared".into(),
        state_path: dir.path().join("leases.json"),
        max_retries: 0,
        ..Config::default()
    })
    .unwrap();
    let iii = Arc::new(register_worker(
        &address,
        InitOptions {
            namespace: Some("provider-project".into()),
            ..InitOptions::default()
        },
    ));
    let delivery = service::register(iii.clone(), manager.clone(), vec!["webhooks".into()]);
    let mut functions = BTreeMap::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while functions.len() < 4 {
            let value = captured.recv().await.unwrap();
            if value["type"] == "registerfunction" {
                functions.insert(value["id"].as_str().unwrap().to_owned(), value);
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        functions.keys().map(String::as_str).collect::<Vec<_>>(),
        vec![
            service::ACQUIRE,
            service::ON_CHANGED_FN,
            service::RELEASE,
            service::STATUS
        ]
    );
    for registration in functions.values() {
        assert!(!registration["description"].as_str().unwrap().is_empty());
        assert_eq!(registration["request_format"]["type"], "object");
        assert_eq!(registration["response_format"]["type"], "object");
    }
    for (function, expected) in [
        (
            service::ACQUIRE,
            serde_json::to_value(schemars::schema_for!(AcquireRequest)).unwrap(),
        ),
        (
            service::RELEASE,
            serde_json::to_value(schemars::schema_for!(ReleaseRequest)).unwrap(),
        ),
        (
            service::STATUS,
            serde_json::to_value(schemars::schema_for!(StatusRequest)).unwrap(),
        ),
    ] {
        assert_eq!(functions[function]["request_format"], expected);
        assert!(functions[function].get("private").is_none());
    }
    // The event source: private, takes no args, returns the snapshot it is
    // called with.
    let changed = &functions[service::ON_CHANGED_FN];
    let snapshot_schema = serde_json::to_value(schemars::schema_for!(Snapshot)).unwrap();
    assert_eq!(changed["private"], true);
    assert!(changed.get("args_format").is_none(), "{changed}");
    assert_eq!(changed["request_format"], snapshot_schema);
    assert_eq!(changed["response_format"], snapshot_schema);
    assert!(changed["response_format"]["properties"]["generation"].is_object());

    // Exercise the registered handlers, not a parallel placeholder catalog.
    for (function, mut data, expect_error) in [
        (service::STATUS, json!({}), false),
        (service::RELEASE, json!({"lease_id":"unknown"}), false),
        (
            service::ACQUIRE,
            json!({"consumer_id":"github", "origin":"http://127.0.0.1:22", "expires_at":"2030-01-01T00:00:00Z"}),
            true,
        ),
    ] {
        data["_caller_worker_id"] = json!("github-worker");
        let reply = invoke(&wire_tx, &mut captured, function, data).await;
        assert_eq!(!reply["error"].is_null(), expect_error, "{reply}");
    }

    // An observer of the source, filtered on the tunnel: the SDK accepts it
    // (no hooks), and each snapshot is then a fire-and-forget call.
    wire_tx
        .send(json!({
            "type": "observeradded",
            "function_id": service::ON_CHANGED_FN,
            "namespace": "provider-project",
            "revision": 1,
            "observer": {
                "id": "github-tunnel",
                "target": "github::on-tunnel-change",
                "filters": { "input.tunnel_id": { "$eq": "webhooks" } },
            },
        }))
        .unwrap();
    let ack = next_matching(&mut captured, |v| v["type"] == "observerack").await;
    assert_eq!(ack["id"], "github-tunnel");
    assert!(ack["error"].is_null(), "{ack}");
    tokio::time::timeout(Duration::from_secs(5), async {
        while !iii.is_observed(service::ON_CHANGED_FN) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    // The snapshots can reach the engine before acquire answers.
    let id = send_invoke(
        &wire_tx,
        service::ACQUIRE,
        json!({"consumer_id":"github", "expires_at":(chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc3339()}),
    );
    let (mut reply, mut call) = (None, None);
    tokio::time::timeout(Duration::from_secs(5), async {
        while reply.is_none() || call.is_none() {
            let value = captured.recv().await.unwrap();
            if value["type"] == "invocationresult" && value["invocation_id"] == id {
                reply = Some(value);
            } else if value["type"] == "invokefunction"
                && value["function_id"] == service::ON_CHANGED_FN
                && call.is_none()
            {
                call = Some(value);
            }
        }
    })
    .await
    .unwrap();
    let (reply, call) = (reply.unwrap(), call.unwrap());
    assert_eq!(reply["result"]["status"], "failed", "{reply}");
    assert!(call["invocation_id"].is_null(), "a void call: {call}");
    assert_eq!(call["action"]["type"], "void");
    assert_eq!(call["data"]["tunnel_id"], "webhooks");
    assert!(call["data"]["generation"].is_string());
    assert!(
        call.get("metadata").is_none(),
        "the engine holds observer metadata"
    );

    manager.shutdown().await;
    delivery.abort();
    let _ = delivery.await;
    tokio::task::spawn_blocking(move || iii.shutdown())
        .await
        .unwrap();
    server.abort();
}

/// Sends a routed call of `function`; returns its invocation id.
fn send_invoke(wire: &mpsc::UnboundedSender<Value>, function: &str, data: Value) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    wire.send(json!({
        "type": "invokefunction",
        "invocation_id": id,
        "function_id": function,
        "data": data,
        "namespace": "provider-project",
    }))
    .unwrap();
    id
}

/// Sends a routed call of `function` and waits for the SDK's answer.
async fn invoke(
    wire: &mpsc::UnboundedSender<Value>,
    captured: &mut mpsc::UnboundedReceiver<Value>,
    function: &str,
    data: Value,
) -> Value {
    let id = send_invoke(wire, function, data);
    next_matching(captured, |v| {
        v["type"] == "invocationresult" && v["invocation_id"] == id
    })
    .await
}
