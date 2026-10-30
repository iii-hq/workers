//! computer's observable surface against a scripted engine, through the hooks
//! the SDK runs: `computer::on-frame-changed` (its added, removed, match and
//! deliver hooks, and the `is_observed_for` gate of the screencast pump) and
//! the registration of `computer::run`, which cannot be observed.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use computer::config::WorkerConfig;
use computer::events::{Emitter, IiiEventSink};
use computer::frames::{
    self, FrameChange, FrameChangeKind, MAX_FRAME_OBSERVERS, ON_FRAME_CHANGED_FN,
};
use computer::functions::{self, catalog};
use computer::session::Sessions;
use futures_util::{SinkExt, StreamExt};
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// A loopback engine that advertises observe.v2, forwards what the test
/// sends, answers every call the worker makes that waits for an answer, and
/// hands every frame the worker sends to the returned receiver.
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
            "worker_id": "computer-test",
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
                    if ws.send(WsMessage::Text(outbound.to_string())).await.is_err() { break }
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
                    let value: Value = serde_json::from_str(&text).unwrap();
                    // The target answers each call through an observer; a void
                    // call has no invocation id.
                    if value["type"] == "invokefunction" && value["invocation_id"].is_string() {
                        let reply = json!({
                            "type": "invocationresult",
                            "invocation_id": value["invocation_id"],
                            "function_id": value["function_id"],
                            "result": { "ok": true },
                        });
                        if ws.send(WsMessage::Text(reply.to_string())).await.is_err() { break }
                    }
                    let _ = captured_tx.send(value);
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
    .expect("expected frame never arrived")
}

/// Frames until `done` holds for those seen, then 300 ms more, so an extra
/// one shows up too.
async fn frames_until(
    rx: &mut mpsc::UnboundedReceiver<Value>,
    done: impl Fn(&[Value]) -> bool,
) -> Vec<Value> {
    let mut seen = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !done(&seen) {
            seen.push(rx.recv().await.unwrap());
        }
    })
    .await
    .unwrap_or_else(|_| panic!("expected frames never arrived: {seen:#?}"));
    let settle = tokio::time::Instant::now() + Duration::from_millis(300);
    while let Ok(Some(value)) = tokio::time::timeout_at(settle, rx.recv()).await {
        seen.push(value);
    }
    seen
}

/// The calls through observers among `frames`: `(observer, change,
/// frame_seq)`, sorted.
fn deliveries(frames: &[Value]) -> Vec<(String, String, u64)> {
    let mut out: Vec<_> = frames
        .iter()
        .filter(|f| f["type"] == "invokefunction" && f["activation"]["observer_id"].is_string())
        .map(|f| {
            let observer = f["activation"]["observer_id"].as_str().unwrap().to_string();
            assert_eq!(
                f["function_id"],
                format!("viewer::{observer}"),
                "a call through an observer reaches its target: {f}"
            );
            assert!(
                f["invocation_id"].is_string(),
                "the slot waits for the target: {f}"
            );
            (
                observer,
                f["data"]["change"].as_str().unwrap().to_string(),
                f["data"]["frame_seq"].as_u64().unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

fn delivered(observer: &str, change: &str, frame_seq: u64) -> (String, String, u64) {
    (observer.to_string(), change.to_string(), frame_seq)
}

/// The routed call's answer among `frames`.
fn answer<'a>(frames: &'a [Value], invocation_id: &str) -> Option<&'a Value> {
    frames
        .iter()
        .find(|f| f["type"] == "invocationresult" && f["invocation_id"] == invocation_id)
}

fn change(session_id: &str, frame_seq: u64, kind: FrameChangeKind) -> FrameChange {
    FrameChange {
        session_id: session_id.to_string(),
        epoch: 7,
        frame_seq,
        change: kind,
        width: 1280,
        height: 800,
        mime: (kind == FrameChangeKind::Updated).then(|| "image/jpeg".to_string()),
        bytes: (kind == FrameChangeKind::Updated).then_some(100_000),
        timestamp: 0,
    }
}

/// Drives the engine's side of `computer::on-frame-changed`.
struct Script {
    wire: mpsc::UnboundedSender<Value>,
    revision: u64,
    calls: u64,
}

impl Script {
    fn next_revision(&mut self) -> u64 {
        self.revision += 1;
        self.revision
    }

    fn add(&mut self, id: &str, session_id: &str, filters: Value) {
        let revision = self.next_revision();
        self.wire
            .send(json!({
                "type": "observeradded",
                "function_id": ON_FRAME_CHANGED_FN,
                "revision": revision,
                "observer": {
                    "id": id,
                    "args": { "session_id": session_id },
                    "filters": filters,
                    "target": format!("viewer::{id}"),
                },
            }))
            .unwrap();
    }

    fn remove(&mut self, id: &str) {
        let revision = self.next_revision();
        self.wire
            .send(json!({
                "type": "observerremoved",
                "function_id": ON_FRAME_CHANGED_FN,
                "revision": revision,
                "id": id,
            }))
            .unwrap();
    }

    /// A call of the function the engine routes to this worker; returns its
    /// invocation id.
    fn route(&mut self, change: &FrameChange) -> String {
        self.calls += 1;
        let invocation_id = format!("00000000-0000-4000-8000-{:012}", self.calls);
        self.wire
            .send(json!({
                "type": "invokefunction",
                "invocation_id": invocation_id,
                "function_id": ON_FRAME_CHANGED_FN,
                "data": change,
            }))
            .unwrap();
        invocation_id
    }
}

async fn ack(rx: &mut mpsc::UnboundedReceiver<Value>, id: &str) -> Value {
    next_matching(rx, |v| v["type"] == "observerack" && v["id"] == id).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frame_changes_reach_the_matching_observers_of_their_session() {
    let (address, wire, mut captured) = engine().await;
    let iii = register_worker(&address, InitOptions::default());
    let notifier = frames::register(&iii);

    let registration = next_matching(&mut captured, |v| {
        v["type"] == "registerfunction" && v["id"] == ON_FRAME_CHANGED_FN
    })
    .await;
    assert_eq!(registration["private"], true, "{registration}");
    assert_eq!(
        registration["args_format"]["required"],
        json!(["session_id"]),
        "{registration}"
    );
    assert!(registration.get("observable").is_none(), "{registration}");

    // The engine's ack: calls of the function from this worker run in-process.
    let mut script = Script {
        wire: wire.clone(),
        revision: 1,
        calls: 0,
    };
    wire.send(json!({
        "type": "functionregistered",
        "id": ON_FRAME_CHANGED_FN,
        "revision": 1,
        "local_dispatch": true,
        "observers": [],
    }))
    .unwrap();
    assert!(!notifier.watching("c1"));

    // Two viewers of c1 (one takes cleared changes only), one of c2.
    script.add("a1", "c1", json!({}));
    script.add(
        "a2",
        "c1",
        json!({ "$and": [{ "input.change": { "$eq": "cleared" } }] }),
    );
    script.add("b1", "c2", json!({}));
    for id in ["a1", "a2", "b1"] {
        let ack = ack(&mut captured, id).await;
        assert!(ack["error"].is_null(), "{ack}");
    }

    // A blank session_id is refused by the added hook.
    script.add("blank", "  ", json!({}));
    let refused = ack(&mut captured, "blank").await;
    assert_eq!(refused["error"]["code"], "INVALID_ARGS", "{refused}");

    // Up to 64 observers; the 65th is refused by the added hook.
    let fillers = MAX_FRAME_OBSERVERS - 3;
    for n in 0..fillers {
        script.add(&format!("f{n}"), "c9", json!({}));
    }
    for n in 0..fillers {
        let ack = ack(&mut captured, &format!("f{n}")).await;
        assert!(ack["error"].is_null(), "{ack}");
    }
    assert_eq!(notifier.observer_count(), MAX_FRAME_OBSERVERS);
    script.add("one-too-many", "c9", json!({}));
    let refused = ack(&mut captured, "one-too-many").await;
    assert_eq!(refused["error"]["code"], "OBSERVER_LIMIT", "{refused}");
    assert_eq!(notifier.observer_count(), MAX_FRAME_OBSERVERS);
    for n in 0..fillers {
        script.remove(&format!("f{n}"));
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        while notifier.observer_count() > 3 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("removed hooks close the fillers' slots");

    // The pump's gate: one lookup by the change's session.
    assert!(notifier.watching("c1"));
    assert!(notifier.watching("c2"));
    assert!(!notifier.watching("c9"));
    assert!(!notifier.watching("nobody"));

    // A change of c1 reaches a1 only: b1 follows another session, and a2's
    // filter takes cleared changes only. The caller gets { offered }.
    let call = script.route(&change("c1", 1, FrameChangeKind::Updated));
    let seen = frames_until(&mut captured, |f| {
        answer(f, &call).is_some() && !deliveries(f).is_empty()
    })
    .await;
    assert_eq!(
        answer(&seen, &call).unwrap()["result"],
        json!({ "offered": 1 })
    );
    assert_eq!(deliveries(&seen), vec![delivered("a1", "updated", 1)]);

    let call = script.route(&change("c1", 1, FrameChangeKind::Cleared));
    let seen = frames_until(&mut captured, |f| {
        answer(f, &call).is_some() && deliveries(f).len() == 2
    })
    .await;
    assert_eq!(
        answer(&seen, &call).unwrap()["result"],
        json!({ "offered": 2 })
    );
    assert_eq!(
        deliveries(&seen),
        vec![delivered("a1", "cleared", 1), delivered("a2", "cleared", 1)]
    );

    let call = script.route(&change("c2", 4, FrameChangeKind::Updated));
    let seen = frames_until(&mut captured, |f| {
        answer(f, &call).is_some() && !deliveries(f).is_empty()
    })
    .await;
    assert_eq!(
        answer(&seen, &call).unwrap()["result"],
        json!({ "offered": 1 })
    );
    assert_eq!(deliveries(&seen), vec![delivered("b1", "updated", 4)]);

    // The pump's own calls run in-process through the same hooks; a change
    // nobody follows is never built into a call.
    notifier.notify(&change("nobody", 1, FrameChangeKind::Updated));
    notifier.notify(&change("c1", 2, FrameChangeKind::Updated));
    let seen = frames_until(&mut captured, |f| !deliveries(f).is_empty()).await;
    assert_eq!(deliveries(&seen), vec![delivered("a1", "updated", 2)]);
    assert!(
        !seen
            .iter()
            .any(|f| f["type"] == "invokefunction" && f["function_id"] == ON_FRAME_CHANGED_FN),
        "an in-process call: {seen:#?}"
    );

    // After observerremoved, a1 receives nothing.
    script.remove("a1");
    tokio::time::timeout(Duration::from_secs(10), async {
        while notifier.slot("a1").is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the removed hook closes a1's slot");
    let call = script.route(&change("c1", 3, FrameChangeKind::Updated));
    let seen = frames_until(&mut captured, |f| answer(f, &call).is_some()).await;
    assert_eq!(
        answer(&seen, &call).unwrap()["result"],
        json!({ "offered": 0 })
    );
    assert_eq!(deliveries(&seen), vec![]);
    let call = script.route(&change("c1", 3, FrameChangeKind::Cleared));
    let seen = frames_until(&mut captured, |f| {
        answer(f, &call).is_some() && !deliveries(f).is_empty()
    })
    .await;
    assert_eq!(
        answer(&seen, &call).unwrap()["result"],
        json!({ "offered": 1 })
    );
    assert_eq!(deliveries(&seen), vec![delivered("a2", "cleared", 3)]);

    script.remove("a2");
    tokio::time::timeout(Duration::from_secs(10), async {
        while notifier.watching("c1") {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("nobody follows c1");
    assert!(notifier.watching("c2"));

    iii.shutdown_async().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn computer_run_cannot_be_observed() {
    let (address, _wire, mut captured) = engine().await;
    let iii: Arc<IIIClient> = Arc::new(register_worker(&address, InitOptions::default()));
    let emitter = Arc::new(Emitter::new(Arc::new(IiiEventSink::new(iii.clone()))));
    let notifier = frames::register(&iii);
    let sessions = Sessions::new(
        WorkerConfig::default().into_shared(),
        emitter,
        notifier,
        iii.clone(),
    );
    functions::register_all(&iii, &sessions);

    let ids: Vec<&str> = catalog().iter().map(|spec| spec.function_id).collect();
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
    .expect("every computer::* function registers");

    for id in ids {
        let registration = &registrations[id];
        if id == functions::RUN_ID {
            assert_eq!(registration["observable"], false, "{registration}");
        } else {
            assert!(registration.get("observable").is_none(), "{registration}");
        }
    }

    iii.shutdown_async().await;
}
