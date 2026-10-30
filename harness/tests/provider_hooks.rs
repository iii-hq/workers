//! The harness's provider functions driven the way the engine drives them:
//! over a raw socket, with `observeradded` / `observerremoved` frames. Each
//! test reads what the harness sends back: `observerack` (accepted or
//! rejected with a code), the `observe` frame of a binding, the
//! `invocationsobserved` report of a call iii delivers, and the calls it makes
//! through observers.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use harness::config::WorkerConfig;
use harness::deps::Deps;
use harness::timer::{TimerBus, MAX_TIMER_MS, ON_TIMER_FN};
use iii_sdk::{register_worker, CallRequest, InitOptions};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio_tungstenite::tungstenite::Message;

mod support;
use support::host;

/// A mock engine: it registers the harness's functions, announces the
/// observers a test pushes, answers every call made to it with `null`, and
/// records every frame the harness sends.
struct Engine {
    deps: Deps,
    timer: TimerBus,
    push: mpsc::UnboundedSender<Value>,
    seen: Arc<Mutex<Vec<Value>>>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.deps.iii.shutdown();
        self.server.abort();
    }
}

impl Engine {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (ready_tx, ready_rx) = oneshot::channel();
        let (push, mut pushed) = mpsc::unbounded_channel::<Value>();
        let recorded = seen.clone();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let mut engine = host::Host::default();
            socket
                .send(Message::Text(host::hello().to_string().into()))
                .await
                .unwrap();
            let _ = ready_tx.send(());
            loop {
                let frame = tokio::select! {
                    Some(message) = pushed.recv() => {
                        if let Some(message) = engine.announce(message) {
                            socket.send(Message::Text(message.to_string().into())).await.unwrap();
                        }
                        continue;
                    }
                    frame = socket.next() => match frame {
                        Some(Ok(frame)) => frame,
                        _ => break,
                    },
                };
                let Ok(message) = serde_json::from_str::<Value>(frame.to_text().unwrap_or(""))
                else {
                    continue;
                };
                recorded.lock().unwrap().push(message.clone());
                if let Some(answer) = engine.answer(&message) {
                    socket
                        .send(Message::Text(answer.to_string().into()))
                        .await
                        .unwrap();
                    continue;
                }
                if message["type"] != "invokefunction" || message["invocation_id"].is_null() {
                    continue;
                }
                let reply = json!({
                    "type": "invocationresult",
                    "invocation_id": message["invocation_id"],
                    "function_id": message["function_id"],
                    "result": null,
                });
                if socket
                    .send(Message::Text(reply.to_string().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let iii = Arc::new(register_worker(&url, InitOptions::default()));
        ready_rx.await.unwrap();
        let deps = Deps::new(
            iii.clone(),
            Arc::new(RwLock::new(Arc::new(WorkerConfig::default()))),
            harness::discovery::new_cell(),
            harness::skills::new_cell(),
            harness::events::TurnEvents::register(&iii),
            harness::hooks::HookRegistry::register(&iii),
        );
        let timer = TimerBus::register(&iii);
        Self {
            deps,
            timer,
            push,
            seen,
            server,
        }
    }

    fn push(&self, frame: Value) {
        self.push.send(frame).unwrap();
    }

    fn frames(&self, wanted: impl Fn(&Value) -> bool) -> Vec<Value> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|frame| wanted(frame))
            .cloned()
            .collect()
    }

    /// The first frame the harness sent that `wanted` accepts.
    async fn frame(&self, what: &str, wanted: impl Fn(&Value) -> bool) -> Value {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(frame) = self.frames(&wanted).into_iter().next() {
                    return frame;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("the harness never sent {what}"))
    }

    /// The harness's `observerack` for the observer `id`.
    async fn ack(&self, id: &str) -> Value {
        self.frame(&format!("an observerack for {id}"), |frame| {
            frame["type"] == "observerack" && frame["id"] == id
        })
        .await
    }

    /// The `invocationsobserved` items of `function_id`'s calls.
    fn observed_calls(&self, function_id: &str) -> Vec<Value> {
        self.frames(|frame| frame["type"] == "invocationsobserved")
            .iter()
            .flat_map(|frame| frame["items"].as_array().cloned().unwrap_or_default())
            .filter(|item| item["function_id"] == function_id)
            .collect()
    }

    /// The function ids the harness called through observers.
    fn called_targets(&self) -> Vec<String> {
        self.frames(|frame| frame["type"] == "invokefunction")
            .iter()
            .filter_map(|frame| frame["function_id"].as_str().map(str::to_string))
            .collect()
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn matched(item: &Value) -> Vec<&str> {
    item["matched"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_event_observer_targeting_spawn_is_forbidden() {
    let engine = Engine::start().await;
    let completed = harness::events::ON_TURN_COMPLETED_FN;
    let started = harness::events::ON_TURN_STARTED_FN;
    engine.push(host::observer_added(
        "completed-spawn",
        completed,
        "harness::spawn",
        json!({}),
        json!({}),
    ));
    engine.push(host::observer_added(
        "completed-ide",
        completed,
        "ide::on-turn",
        json!({}),
        json!({ "input.session_id": { "$eq": "s1" } }),
    ));
    engine.push(host::observer_added(
        "started-spawn",
        started,
        "harness::spawn",
        json!({}),
        json!({}),
    ));
    let refused = engine.ack("completed-spawn").await;
    assert_eq!(refused["error"]["code"], json!("FORBIDDEN"), "{refused}");
    let accepted = engine.ack("completed-ide").await;
    assert!(accepted.get("error").is_none(), "{accepted}");
    let refused = engine.ack("started-spawn").await;
    assert_eq!(refused["error"]["code"], json!("FORBIDDEN"), "{refused}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_timer_past_seven_days_is_refused_and_the_last_removal_cancels_its_one_shot() {
    let engine = Engine::start().await;
    let now = now_ms();
    engine.push(host::observer_added(
        "far",
        ON_TIMER_FN,
        "test::far",
        json!({ "at": now + MAX_TIMER_MS + 60_000 }),
        json!({}),
    ));
    let refused = engine.ack("far").await;
    assert_eq!(refused["error"]["code"], json!("CONFIG_ERROR"), "{refused}");
    assert_eq!(engine.timer.armed_count(), 0);

    // Hooks of one function run in order, so each ack below follows every
    // frame pushed before it.
    let (at, later, latest) = (now + 600_000, now + 700_000, now + 800_000);
    for id in ["a", "b"] {
        engine.push(host::observer_added(
            id,
            ON_TIMER_FN,
            "test::wake",
            json!({ "at": at }),
            json!({}),
        ));
    }
    assert!(engine.ack("a").await.get("error").is_none());
    assert!(engine.ack("b").await.get("error").is_none());
    assert_eq!(engine.timer.armed_count(), 1, "one one-shot per instant");

    engine.push(host::observer_removed("a", ON_TIMER_FN));
    engine.push(host::observer_added(
        "c",
        ON_TIMER_FN,
        "test::wake",
        json!({ "at": later }),
        json!({}),
    ));
    engine.ack("c").await;
    assert_eq!(
        engine.timer.armed_count(),
        2,
        "`b` still observes the first instant"
    );

    engine.push(host::observer_removed("b", ON_TIMER_FN));
    engine.push(host::observer_added(
        "d",
        ON_TIMER_FN,
        "test::wake",
        json!({ "at": latest }),
        json!({}),
    ));
    engine.ack("d").await;
    assert_eq!(
        engine.timer.armed_count(),
        2,
        "the first instant's last observer took its one-shot with it"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_observers_of_a_past_instant_share_one_call() {
    let engine = Engine::start().await;
    let at = now_ms() - 1_000;
    for id in ["late-1", "late-2"] {
        engine.push(host::observer_added(
            id,
            ON_TIMER_FN,
            "test::wake",
            json!({ "at": at }),
            json!({}),
        ));
    }
    assert!(engine.ack("late-1").await.get("error").is_none());
    assert!(engine.ack("late-2").await.get("error").is_none());
    engine
        .frame("the late timer call", |frame| {
            frame["type"] == "invocationsobserved"
                && frame["items"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|i| i["function_id"] == ON_TIMER_FN))
        })
        .await;
    // Room for a second call, were there one.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let calls = engine.observed_calls(ON_TIMER_FN);
    assert_eq!(calls.len(), 1, "one call for the instant: {calls:?}");
    let mut reached = matched(&calls[0]);
    reached.sort_unstable();
    assert_eq!(reached, vec!["late-1", "late-2"]);
    assert_eq!(calls[0]["input"]["scheduled_at"], json!(at));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_binding_observer_carries_its_id_pointer_and_expiry_backstop() {
    use harness::bindings::{Binding, BindingTarget, Lifecycle, OwnerScope, SourceSpec};
    let engine = Engine::start().await;
    let expires_at = now_ms() + 60_000;
    let binding = Binding {
        id: "sub_frame".into(),
        observer_id: None,
        source: Some(SourceSpec {
            function_id: "state::on-changed".into(),
            namespace: None,
            args: None,
            filters: Some(json!({ "input.scope": { "$eq": "run" } })),
        }),
        owner: OwnerScope {
            session_id: "s1".into(),
            root_session_id: None,
        },
        target: BindingTarget::new("harness::send"),
        conditions: vec![],
        lifecycle: Lifecycle {
            once: true,
            max_fires: None,
            expires_at: Some(expires_at),
        },
        capability: None,
        dedup_key: None,
        fires: 0,
        created_at: 0,
    };
    harness::bindings::observe_binding(&engine.deps, &binding).expect("a binding with a source");
    let frame = engine
        .frame("the binding's observe frame", |frame| {
            frame["type"] == "observe" && frame["id"] == "sub_frame"
        })
        .await;
    assert_eq!(frame["metadata"], json!({ "__binding": "sub_frame" }));
    assert_eq!(
        frame["target"]["function_id"],
        json!(harness::functions::observer_deliver::DELIVER_ID)
    );
    assert_eq!(frame["source"]["function_id"], json!("state::on-changed"));
    assert_eq!(
        frame["source"]["filters"],
        json!({ "input.scope": { "$eq": "run" } })
    );
    assert!(
        frame.get("max_fires").is_none(),
        "the harness counts fires: {frame}"
    );
    // The backstop: the binding's remaining lifetime plus a minute's grace.
    let expires_in = frame["expires_in_ms"].as_u64().expect("an expires_in_ms");
    assert!(
        (110_000..=120_000).contains(&expires_in),
        "expires_in_ms {expires_in}"
    );
    assert!(engine.deps.observer_handles.contains("sub_frame"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hook_observer_whose_filters_miss_the_call_is_not_called() {
    let engine = Engine::start().await;
    let point = harness::hooks::HookPoint::PreTurn;
    for (id, target, session) in [("hit", "test::hit", "s1"), ("miss", "test::miss", "s2")] {
        engine.push(host::observer_added(
            id,
            point.function_id(),
            target,
            json!({}),
            json!({ "input.session_id": { "$eq": session } }),
        ));
    }
    engine.ack("hit").await;
    engine.ack("miss").await;
    let input = json!({
        "point": point.input_name(),
        "session_id": "s1",
        "turn_id": "t1",
        "step": 0,
        "depth": 0,
    });
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        engine
            .deps
            .iii
            .call(CallRequest::new(point.function_id(), input)),
    )
    .await
    .expect("the chain answers")
    .expect("the chain runs");
    assert_eq!(outcome["decision"], json!("continue"));
    let called = engine.called_targets();
    assert!(called.iter().any(|t| t == "test::hit"), "{called:?}");
    assert!(!called.iter().any(|t| t == "test::miss"), "{called:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn static_injections_follow_session_globs_and_filters() {
    let engine = Engine::start().await;
    let point = harness::hooks::HookPoint::PreGenerate;
    for (id, args, filters) in [
        ("everywhere", json!({ "inject_prompt": "E" }), json!({})),
        (
            "glob",
            json!({ "inject_prompt": "G", "sessions": ["s1*"] }),
            json!({}),
        ),
        (
            "filtered",
            json!({ "inject_prompt": "F" }),
            json!({ "input.session_id": { "$eq": "s2" } }),
        ),
        ("dynamic", json!({}), json!({})),
    ] {
        engine.push(host::observer_added(
            id,
            point.function_id(),
            &format!("test::{id}"),
            args,
            filters,
        ));
    }
    for id in ["everywhere", "glob", "filtered", "dynamic"] {
        engine.ack(id).await;
    }
    let injected = |session: &str| -> Vec<String> {
        engine
            .deps
            .hooks
            .static_injections(&json!({
                "point": point.input_name(),
                "session_id": session,
                "turn_id": "t",
                "step": 0,
                "depth": 0,
            }))
            .into_iter()
            .filter_map(|binding| binding.inject_prompt)
            .collect()
    };
    let mut s1 = injected("s1-child");
    s1.sort();
    assert_eq!(s1, vec!["E", "G"]);
    let mut s2 = injected("s2");
    s2.sort();
    assert_eq!(s2, vec!["E", "F"]);
    assert_eq!(injected("s3"), vec!["E"]);
}
