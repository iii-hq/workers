//! `tmux::on-changed` against a scripted engine: the registration, the first
//! overview sent through a new observer, and the poll loop's calls while the
//! function is observed: one per change, none for the state the new observer
//! already received.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iii_sdk::{register_worker, InitOptions};
use serde_json::{json, Value};
use tmux::collector::{Collector, ConsoleClients};
use tmux::config::WorkerConfig;
use tmux::events::{self, Monitor, ON_CHANGED_FN};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;

const TARGET: &str = "iii::tmux-ui::changed::console-test";

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
            "worker_id": "tmux-test",
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

async fn next_matching(
    rx: &mut mpsc::UnboundedReceiver<Value>,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
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

/// Fails on any frame `predicate` matches during `within`.
async fn none_matching(
    rx: &mut mpsc::UnboundedReceiver<Value>,
    within: Duration,
    predicate: impl Fn(&Value) -> bool,
) {
    let deadline = tokio::time::Instant::now() + within;
    while let Ok(Some(value)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        assert!(!predicate(&value), "unexpected frame: {value}");
    }
}

fn is_feed_call(frame: &Value) -> bool {
    frame["type"] == "invokefunction" && frame["function_id"] == ON_CHANGED_FN
}

fn is_overview(data: &Value) -> bool {
    data["sessions"]["sessions"].is_array() && data["ports"]["listeners"].is_array()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn observers_get_the_current_overview_and_every_change() {
    let (address, wire, mut captured) = engine().await;
    let iii = register_worker(&address, InitOptions::default());
    // A server nobody else uses: the overview is empty, or reports tmux
    // missing. Listening ports are scanned once, so the host's own ports
    // coming and going do not change the overview during the test.
    let base = WorkerConfig {
        socket_name: Some(format!("iii-tmux-feed-{}", uuid::Uuid::new_v4().simple())),
        poll_interval_ms: 50,
        ports_interval_ms: 3_600_000,
        ..WorkerConfig::default()
    };
    let config = base.clone().into_shared();
    let collector = Arc::new(Collector::new(config.clone(), ConsoleClients::default()));
    let monitor = Monitor::new(iii.clone(), collector);
    events::register(&iii, monitor.clone());
    monitor.spawn();

    let registration = next_matching(&mut captured, |v| {
        v["type"] == "registerfunction" && v["id"] == ON_CHANGED_FN
    })
    .await;
    assert_eq!(registration["private"], true);
    assert!(
        registration.get("args_format").is_none(),
        "takes no args: {registration}"
    );
    assert!(registration["response_format"]["properties"]["sessions"].is_object());
    assert!(registration["response_format"]["properties"]["ports"].is_object());
    assert!(!iii.is_observed(ON_CHANGED_FN));

    wire.send(json!({
        "type": "observeradded",
        "function_id": ON_CHANGED_FN,
        "revision": 1,
        "observer": { "id": "page", "target": TARGET, "filters": {} },
    }))
    .unwrap();
    let ack = next_matching(&mut captured, |v| v["type"] == "observerack").await;
    assert_eq!(ack["id"], "page");
    assert!(ack["error"].is_null(), "{ack}");

    // The new observer alone receives the current overview, through itself,
    // before any call of the function.
    let first = next_matching(&mut captured, |v| {
        assert!(!is_feed_call(v), "a call before the first overview: {v}");
        v["type"] == "invokefunction" && v["activation"]["observer_id"] == "page"
    })
    .await;
    assert_eq!(first["function_id"], TARGET);
    assert_eq!(first["action"]["type"], "void");
    assert!(is_overview(&first["data"]), "{first}");

    // The poll loop runs every 50 ms while observed, and does not call the
    // function again with the overview the observer already has.
    none_matching(&mut captured, Duration::from_millis(600), is_feed_call).await;

    // A change: the poll loop calls the function with the new overview, once.
    let missing = format!("iii-tmux-missing-{}", uuid::Uuid::new_v4().simple());
    config.store(Arc::new(WorkerConfig {
        tmux_binary: missing.clone(),
        ..base
    }));
    monitor.poke();
    let call = next_matching(&mut captured, is_feed_call).await;
    assert!(call["invocation_id"].is_null(), "a void call: {call}");
    assert!(is_overview(&call["data"]), "{call}");
    let error = call["data"]["sessions"]["tmux"]["error"]
        .as_str()
        .unwrap_or_default();
    assert!(error.contains(&missing), "{call}");
    none_matching(&mut captured, Duration::from_millis(300), is_feed_call).await;

    wire.send(json!({
        "type": "observerremoved",
        "function_id": ON_CHANGED_FN,
        "revision": 2,
        "id": "page",
    }))
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while iii.is_observed(ON_CHANGED_FN) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    tokio::task::spawn_blocking(move || iii.shutdown())
        .await
        .unwrap();
}
