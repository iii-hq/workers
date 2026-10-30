//! The four observable functions against a scripted engine: their
//! registrations, and the keys `ios-simulator::on-frame-event` files its
//! observers under, as the bridge's `frame_observed` gate reads them.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iii_sdk::{register_worker, IIIClient, InitOptions};
use ios_simulator::events::{
    Events, FrameArgs, ON_ACTIVITY_FN, ON_DEVICE_CHANGED_FN, ON_FRAME_EVENT_FN, ON_MEDIA_CHANGED_FN,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// A loopback engine that advertises observe.v2, forwards what the test
/// sends, and hands every frame the worker sends to `captured`.
async fn engine() -> (
    String,
    mpsc::UnboundedSender<Value>,
    mpsc::UnboundedReceiver<Value>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("ws://{}", listener.local_addr().unwrap());
    let (captured_tx, captured) = mpsc::unbounded_channel::<Value>();
    let (wire_tx, mut wire_rx) = mpsc::unbounded_channel::<Value>();
    wire_tx
        .send(json!({
            "type": "workerregistered",
            "worker_id": "ios-simulator-test",
            "features": ["observe.v1", "observe.v2"],
        }))
        .unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        loop {
            tokio::select! {
                outbound = wire_rx.recv() => {
                    let Some(outbound) = outbound else { break };
                    if ws.send(WsMessage::Text(outbound.to_string().into())).await.is_err() { break }
                }
                inbound = ws.next() => {
                    let text = match inbound {
                        Some(Ok(WsMessage::Text(text))) => text,
                        Some(Ok(WsMessage::Ping(data))) => {
                            if ws.send(WsMessage::Pong(data)).await.is_err() { break }
                            continue;
                        }
                        Some(Ok(WsMessage::Pong(_))) => continue,
                        _ => break,
                    };
                    let _ = captured_tx.send(serde_json::from_str(&text).unwrap());
                }
            }
        }
    });
    (address, wire_tx, captured)
}

async fn ack(rx: &mut mpsc::UnboundedReceiver<Value>, id: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let value = rx.recv().await.unwrap();
            if value["type"] == "observerack" && value["id"] == id {
                break value;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no ack of {id}"))
}

async fn until(what: &str, condition: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frames_are_observed_by_tenant_and_udid() {
    let (address, wire, mut captured) = engine().await;
    let iii: Arc<IIIClient> = Arc::new(register_worker(&address, InitOptions::default()));
    let events = Events::register(&iii);

    let ids = [
        ON_DEVICE_CHANGED_FN,
        ON_MEDIA_CHANGED_FN,
        ON_ACTIVITY_FN,
        ON_FRAME_EVENT_FN,
    ];
    let mut registrations = BTreeMap::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !ids.iter().all(|id| registrations.contains_key(*id)) {
            let frame = captured.recv().await.unwrap();
            if frame["type"] == "registerfunction" {
                registrations.insert(frame["id"].as_str().unwrap().to_string(), frame);
            }
        }
    })
    .await
    .expect("the four observable functions register");

    // Each is private; only frames take args, and the engine validates them
    // against FrameArgs.
    let frame_args = serde_json::to_value(schemars::schema_for!(FrameArgs)).unwrap();
    for id in ids {
        let registration = &registrations[id];
        assert_eq!(registration["private"], true, "{registration}");
        assert!(registration.get("observable").is_none(), "{registration}");
        if id == ON_FRAME_EVENT_FN {
            assert_eq!(
                registration["args_format"]["properties"],
                frame_args["properties"]
            );
            assert_eq!(registration["args_format"]["additionalProperties"], false);
        } else {
            assert!(registration.get("args_format").is_none(), "{registration}");
        }
    }

    // Nothing observes frames: none is built.
    assert!(!events.frame_observed("acme", "U1"));

    let mut revision = 0;
    let mut add = |id: &str, args: Value| {
        revision += 1;
        wire.send(json!({
            "type": "observeradded",
            "function_id": ON_FRAME_EVENT_FN,
            "revision": revision,
            "observer": { "id": id, "args": args, "filters": {}, "target": "viewer::frames" },
        }))
        .unwrap();
    };

    // One tenant: every simulator of acme, nothing of globex.
    add("acme", json!({ "tenant": "acme" }));
    assert!(ack(&mut captured, "acme").await["error"].is_null());
    until("acme's observer", || events.frame_observed("acme", "U1")).await;
    assert!(events.frame_observed("acme", "U2"));
    assert!(!events.frame_observed("globex", "U1"));

    // One udid, any tenant.
    add("u2", json!({ "udid": "U2" }));
    assert!(ack(&mut captured, "u2").await["error"].is_null());
    until("u2's observer", || events.frame_observed("globex", "U2")).await;
    assert!(!events.frame_observed("globex", "U3"));

    // One tenant's simulator.
    add("globex-u3", json!({ "tenant": "globex", "udid": "U3" }));
    assert!(ack(&mut captured, "globex-u3").await["error"].is_null());
    until("globex-u3's observer", || {
        events.frame_observed("globex", "U3")
    })
    .await;
    assert!(!events.frame_observed("globex", "U4"));
    assert!(!events.frame_observed("initech", "U3"));

    // Removing acme's observer stops acme's frames.
    revision += 1;
    wire.send(json!({
        "type": "observerremoved",
        "function_id": ON_FRAME_EVENT_FN,
        "revision": revision,
        "id": "acme",
    }))
    .unwrap();
    until("acme's observer removed", || {
        !events.frame_observed("acme", "U1")
    })
    .await;
    assert!(events.frame_observed("acme", "U2"));

    // The operator's console: no args, every tenant and simulator.
    revision += 1;
    wire.send(json!({
        "type": "observeradded",
        "function_id": ON_FRAME_EVENT_FN,
        "revision": revision,
        "observer": { "id": "console", "args": {}, "filters": {}, "target": "viewer::frames" },
    }))
    .unwrap();
    assert!(ack(&mut captured, "console").await["error"].is_null());
    until("the console's observer", || {
        events.frame_observed("initech", "U9")
    })
    .await;

    iii.shutdown_async().await;
}
