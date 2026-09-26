use futures_util::{SinkExt, StreamExt};
use iii_sdk::{
    register_worker,
    trigger::{TriggerConfig, TriggerHandler},
    InitOptions, Message,
};
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
async fn interface_registers_without_cloudflared_and_sdk_preserves_namespace_metadata() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("ws://{}", listener.local_addr().unwrap());
    let (captured_tx, mut captured) = mpsc::unbounded_channel::<Value>();
    let (wire_tx, mut wire_rx) = mpsc::unbounded_channel::<Message>();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        loop {
            tokio::select! {
                outbound = wire_rx.recv() => {
                    let Some(outbound) = outbound else { break; };
                    if ws.send(WsMessage::Text(serde_json::to_string(&outbound).unwrap().into())).await.is_err() { break; }
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
                    if let Ok(Message::InvokeFunction { invocation_id: Some(id), function_id, .. }) = serde_json::from_str(&text) {
                        let reply = Message::InvocationResult { invocation_id: id, function_id, result: Some(json!({"ok":true})), error: None, traceparent: None, baggage: None };
                        ws.send(WsMessage::Text(serde_json::to_string(&reply).unwrap().into())).await.unwrap();
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
    let mut trigger = None;
    tokio::time::timeout(Duration::from_secs(5), async {
        while functions.len() < 3 || trigger.is_none() {
            let value = captured.recv().await.unwrap();
            match value["type"].as_str() {
                Some("registerfunction") => {
                    functions.insert(value["id"].as_str().unwrap().to_owned(), value);
                }
                Some("registertriggertype") => trigger = Some(value),
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        functions.keys().map(String::as_str).collect::<Vec<_>>(),
        vec![service::ACQUIRE, service::RELEASE, service::STATUS]
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
    }
    let trigger = trigger.unwrap();
    assert_eq!(trigger["id"], service::CHANGED);
    assert!(trigger["trigger_request_format"]["properties"]["tunnel_id"].is_object());
    assert!(trigger["call_request_format"]["properties"]["generation"].is_object());

    // Exercise the registered handlers, not a parallel placeholder catalog.
    for (function, mut data, expect_error) in [
        (service::STATUS, json!({}), false),
        (
            service::ACQUIRE,
            json!({"consumer_id":"github", "expires_at":(chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc3339()}),
            false,
        ),
        (
            service::ACQUIRE,
            json!({"consumer_id":"github", "origin":"http://127.0.0.1:22", "expires_at":"2030-01-01T00:00:00Z"}),
            true,
        ),
        (service::RELEASE, json!({"lease_id":"unknown"}), false),
    ] {
        data["_caller_worker_id"] = json!("github-worker");
        let id = uuid::Uuid::new_v4();
        wire_tx
            .send(Message::InvokeFunction {
                invocation_id: Some(id),
                function_id: function.into(),
                data,
                traceparent: None,
                baggage: None,
                action: None,
                metadata: None,
                namespace: Some("provider-project".into()),
            })
            .unwrap();
        let reply = next_matching(&mut captured, |v| {
            v["type"] == "invocationresult" && v["invocation_id"] == id.to_string()
        })
        .await;
        assert_eq!(!reply["error"].is_null(), expect_error, "{reply}");
        if function == service::ACQUIRE && !expect_error {
            assert_eq!(reply["result"]["status"], "failed");
        }
    }

    let changes = service::Changes::new(vec!["webhooks".into()]);
    let mut binding = TriggerConfig {
        id: "subscription".into(),
        function_id: "github::on-tunnel-change".into(),
        config: json!({"tunnel_id":"webhooks"}),
        metadata: Some(json!({"payload":{"repo":"owner/repo"}, "event_into":"/event"})),
        namespace: Some("github-project".into()),
    };
    changes.register_trigger(binding.clone()).await.unwrap();
    let snapshot = Snapshot {
        tunnel_id: "webhooks".into(),
        status: Status::Ready,
        public_url: Some("https://one.trycloudflare.com".into()),
        generation: "unique".into(),
        error: None,
    };
    iii.trigger(service::routed_request(&binding, &snapshot).unwrap())
        .await
        .unwrap();
    let invocation = next_matching(&mut captured, |v| {
        v["type"] == "invokefunction" && v["function_id"] == binding.function_id
    })
    .await;
    assert_eq!(invocation["metadata"], binding.metadata.clone().unwrap());
    assert_eq!(invocation["namespace"], "github-project");
    assert_eq!(invocation["data"]["generation"], "unique");
    binding.namespace = None;
    iii.trigger(service::routed_request(&binding, &snapshot).unwrap())
        .await
        .unwrap();
    let invocation = next_matching(&mut captured, |v| {
        v["type"] == "invokefunction" && v["function_id"] == binding.function_id
    })
    .await;
    assert_eq!(invocation["namespace"], "default");
    binding.config = Value::Null;
    changes.unregister_trigger(binding).await.unwrap();
    manager.shutdown().await;
    delivery.abort();
    let _ = delivery.await;
    tokio::task::spawn_blocking(move || iii.shutdown())
        .await
        .unwrap();
    server.abort();
}
