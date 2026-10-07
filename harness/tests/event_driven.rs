//! No polling: binding deadlines and orphaned-turn recovery run on their own
//! timers and on the engine's worker change feed, never on a periodic scan.
//! Production handlers run against an isolated mock SDK transport that can
//! also push engine-side invocations (the `engine::workers-available` fire).
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use harness::bindings::{Binding, BindingTarget, Causation, Lifecycle, OwnerScope};
use harness::config::WorkerConfig;
use harness::deps::Deps;
use harness::types::message::AgentMessage;
use harness::types::turn::FunctionPolicy;
use iii_sdk::{register_worker, InitOptions};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio_tungstenite::tungstenite::Message as Frame;

#[derive(Default)]
struct Store {
    state: BTreeMap<(String, String), Value>,
    /// Every worker -> engine invocation: `(function_id, data, action)`.
    calls: Vec<(String, Value, Value)>,
    /// While set, `sink::record` replies are parked here until released —
    /// a dispatch held mid-flight, between a fire's claim and its retirement.
    hold_sink: bool,
    held: Vec<Value>,
}

impl Store {
    fn get(&self, scope: &str, key: &str) -> Value {
        self.state
            .get(&(scope.into(), key.into()))
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn put(&mut self, scope: &str, key: &str, value: Value) {
        if value.is_null() {
            self.state.remove(&(scope.into(), key.into()));
        } else {
            self.state.insert((scope.into(), key.into()), value);
        }
    }

    fn respond(&mut self, function: &str, data: &Value, action: &Value) -> Result<Value, String> {
        self.calls
            .push((function.into(), data.clone(), action.clone()));
        if action["type"] == "enqueue" {
            return Ok(json!({ "message_receipt_id": "receipt" }));
        }
        let scope = data["scope"].as_str().unwrap_or_default();
        let key = data["key"].as_str().unwrap_or_default();
        match function {
            "state::get" | "harness::state::get" => Ok(self.get(scope, key)),
            "state::set" | "harness::state::set" => {
                self.put(scope, key, data["value"].clone());
                Ok(json!({}))
            }
            "state::delete" | "harness::state::delete" => {
                self.put(scope, key, Value::Null);
                Ok(json!({}))
            }
            "state::list_keys" => Ok(json!({ "keys": self
                .state
                .keys()
                .filter(|(s, _)| s == scope)
                .map(|(_, k)| k.clone())
                .collect::<Vec<_>>() })),
            "state::list" | "harness::state::list" => Ok(Value::Array(
                self.state
                    .iter()
                    .filter(|((s, _), _)| s == scope)
                    .map(|(_, v)| v.clone())
                    .collect(),
            )),
            "harness::state::compare-and-set" => {
                let current = self.get(scope, key);
                let swapped = current == data["expected"];
                if swapped {
                    self.put(scope, key, data["value"].clone());
                }
                Ok(json!({ "swapped": swapped, "current": current }))
            }
            "state::claim-namespace" => Ok(json!({ "claimed": true })),
            "session::get" => Ok(json!({ "meta": {
                "session_id": data["session_id"], "title": "owner", "metadata": {}
            }})),
            "session::append" => Ok(json!({ "entry_id": data["entry_id"] })),
            "approval::evaluate" => Ok(json!({ "verdict": "allow" })),
            "sink::record" => Ok(json!({ "recorded": true })),
            "engine::workers::register" => Ok(json!({ "success": true })),
            _ => Err(format!("unexpected mock RPC {function}")),
        }
    }

    fn called(&self, function: &str) -> usize {
        self.calls.iter().filter(|(f, _, _)| f == function).count()
    }

    fn appended(&self, entry_id: &str) -> bool {
        self.calls
            .iter()
            .any(|(f, data, _)| f == "session::append" && data["entry_id"] == entry_id)
    }
}

struct Stack {
    deps: Arc<Deps>,
    store: Arc<Mutex<Store>>,
    /// Engine -> worker frames (an engine-side fire of a bound trigger).
    push: mpsc::UnboundedSender<Value>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Stack {
    fn drop(&mut self) {
        self.deps.iii.shutdown();
        self.server.abort();
    }
}

impl Stack {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let store = Arc::new(Mutex::new(Store::default()));
        let (push, mut pushed) = mpsc::unbounded_channel::<Value>();
        let (ready_tx, ready_rx) = oneshot::channel();
        let state = store.clone();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let (mut tx, mut rx) = socket.split();
            let _ = ready_tx.send(());
            loop {
                tokio::select! {
                    Some(message) = pushed.recv() => {
                        if tx.send(Frame::Text(message.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    frame = rx.next() => {
                        let Some(Ok(frame)) = frame else { break };
                        let Ok(text) = frame.to_text() else { continue };
                        let Ok(message) = serde_json::from_str::<Value>(text) else { continue };
                        if message["type"] != "invokefunction" {
                            continue;
                        }
                        let function = message["function_id"].as_str().unwrap_or_default().to_string();
                        let result = state
                            .lock()
                            .unwrap()
                            .respond(&function, &message["data"], &message["action"]);
                        if message["invocation_id"].is_null() {
                            continue;
                        }
                        let mut reply = json!({
                            "type": "invocationresult",
                            "invocation_id": message["invocation_id"],
                            "function_id": function,
                        });
                        match result {
                            Ok(value) => reply["result"] = value,
                            Err(error) => reply["error"] = json!({ "code": "test_error", "message": error }),
                        }
                        {
                            let mut store = state.lock().unwrap();
                            if function == "sink::record" && store.hold_sink {
                                store.held.push(reply);
                                continue;
                            }
                        }
                        if tx.send(Frame::Text(reply.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let iii = Arc::new(register_worker(&url, InitOptions::default()));
        ready_rx.await.unwrap();
        let cfg = WorkerConfig {
            session_timeout_ms: 2_000,
            dispatch_timeout_ms: 2_000,
            ..WorkerConfig::default()
        };
        let deps = Arc::new(Deps::new(
            iii.clone(),
            Arc::new(RwLock::new(Arc::new(cfg))),
            harness::discovery::new_cell(),
            harness::skills::new_cell(),
            harness::events::TurnEvents::register(&iii),
            harness::hooks::HookRegistry::register(&iii),
        ));
        Self {
            deps,
            store,
            push,
            server,
        }
    }

    fn insert_binding(&self, binding: &Binding) {
        self.store.lock().unwrap().put(
            "harness_binding",
            &binding.id,
            serde_json::to_value(binding).unwrap(),
        );
    }

    fn release_held(&self) {
        let mut store = self.store.lock().unwrap();
        store.hold_sink = false;
        for reply in store.held.drain(..) {
            self.push.send(reply).unwrap();
        }
    }

    fn trigger_records(&self, binding: &str) -> Vec<String> {
        self.store
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(f, data, _)| {
                f == "session::append" && data["custom"]["custom_type"] == "trigger_fired"
            })
            .filter(|(_, data, _)| data["custom"]["data"]["subscription_id"] == binding)
            .map(|(_, data, _)| data["entry_id"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    fn binding_stored(&self, id: &str) -> bool {
        !self
            .store
            .lock()
            .unwrap()
            .get("harness_binding", id)
            .is_null()
    }

    async fn wait_for(&self, what: &str, done: impl Fn(&Store) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if done(&self.store.lock().unwrap()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let calls: Vec<String> = self
            .store
            .lock()
            .unwrap()
            .calls
            .iter()
            .map(|(f, _, _)| f.clone())
            .collect();
        panic!("timed out waiting for {what}; calls: {calls:?}");
    }
}

/// A mechanical reaction (call target), so retirement records without the
/// wake path's message injection.
fn call_binding(id: &str, once: bool, expires_at: i64) -> Binding {
    Binding {
        id: id.into(),
        trigger_id: Some(format!("sdk:{id}")),
        owner: OwnerScope {
            session_id: "s_owner".into(),
            root_session_id: None,
        },
        target: BindingTarget::new("sink::record"),
        conditions: vec![],
        lifecycle: Lifecycle {
            once,
            max_fires: None,
            expires_at: Some(expires_at),
        },
        capability: Some(FunctionPolicy {
            allow: vec!["sink::*".into()],
            deny: vec![],
            expose: Default::default(),
        }),
        causation: Causation::default(),
        dedup_key: Some(json!({
            "trigger_type": "state",
            "config": { "scope": "watched", "key": "k" },
        })),
        fires: 0,
        created_at: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_binding_is_retired_by_its_own_timer_without_any_sweep() {
    let stack = Stack::new().await;
    let armed_at = Instant::now();
    let binding = call_binding("sub_deadline", false, AgentMessage::now_ms() + 400);
    stack.insert_binding(&binding);

    harness::bindings::expiry::arm(&stack.deps, &binding);
    assert!(stack.deps.expiry_timers.is_armed("sub_deadline"));

    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        stack.binding_stored("sub_deadline"),
        "nothing may retire a binding before its deadline"
    );

    stack
        .wait_for("the expiry record", |store| {
            store.appended("e_trigexpired_sub_deadline")
        })
        .await;
    let waited = armed_at.elapsed();
    assert!(
        waited >= Duration::from_millis(380),
        "retired after {waited:?}, before its deadline"
    );
    assert!(
        !stack.binding_stored("sub_deadline"),
        "deleted before the notice"
    );
    assert!(
        stack.deps.expiry_timers.is_empty(),
        "the timer detached itself"
    );
    let store = stack.store.lock().unwrap();
    assert_eq!(
        store.called("harness::state::list") + store.called("state::list"),
        0,
        "the deadline fired on its own timer; nothing listed the store"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_binding_that_fires_before_its_deadline_cancels_its_timer() {
    let stack = Stack::new().await;
    let binding = call_binding("sub_fired", true, AgentMessage::now_ms() + 60_000);
    stack.insert_binding(&binding);
    harness::bindings::expiry::arm(&stack.deps, &binding);
    assert!(stack.deps.expiry_timers.is_armed("sub_fired"));

    let result = harness::functions::trigger_deliver::handle(
        &stack.deps,
        json!({ "type": "state", "scope": "watched", "key": "k", "new_value": 1 }),
        Some(json!({ "__binding": "sub_fired" })),
    )
    .await
    .unwrap();

    assert!(result.delivered, "{result:?}");
    assert_eq!(stack.store.lock().unwrap().called("sink::record"), 1);
    assert!(
        !stack.binding_stored("sub_fired"),
        "the once fire retired it"
    );
    assert!(
        !stack.deps.expiry_timers.is_armed("sub_fired"),
        "retiring the record must cancel its deadline"
    );
    assert!(!stack
        .store
        .lock()
        .unwrap()
        .appended("e_trigexpired_sub_fired"));
}

/// The e2e regression: a once Compose wake's terminal event reached the
/// harness twice — the binding's own trigger and a recovery replay (the
/// standing terminal watch). The replay landed between the live delivery's
/// claim and its retirement, found the binding spent, retired it itself and
/// wrote a "skipped / binding exhausted" record next to the real delivery.
/// A spent-budget duplicate must stay invisible: one dispatch, one record.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_duplicate_fire_during_delivery_is_not_a_second_outcome() {
    let stack = Stack::new().await;
    let binding = call_binding("sub_once", true, AgentMessage::now_ms() + 60_000);
    stack.insert_binding(&binding);
    stack.store.lock().unwrap().hold_sink = true;
    let event = json!({ "operation_id": "add-todo-app-1", "terminal": true, "sequence": 40 });

    let live = {
        let deps = stack.deps.clone();
        let event = event.clone();
        tokio::spawn(async move {
            harness::functions::trigger_deliver::handle(
                &deps,
                event,
                Some(json!({ "__binding": "sub_once" })),
            )
            .await
        })
    };
    // The live delivery has claimed its slot and is mid-dispatch.
    stack
        .wait_for("the live dispatch", |store| {
            store.called("sink::record") == 1
        })
        .await;

    let replay = harness::functions::trigger_deliver::handle(
        &stack.deps,
        event,
        Some(json!({ "__binding": "sub_once" })),
    )
    .await
    .unwrap();
    assert!(!replay.delivered, "{replay:?}");
    assert!(
        stack.trigger_records("sub_once").is_empty(),
        "the duplicate wrote an outcome: {:?}",
        stack.trigger_records("sub_once")
    );
    assert!(
        stack.binding_stored("sub_once"),
        "the claiming delivery owns retirement"
    );

    stack.release_held();
    let delivered = live.await.unwrap().unwrap();
    assert!(delivered.delivered, "{delivered:?}");
    assert_eq!(stack.store.lock().unwrap().called("sink::record"), 1);
    assert!(!stack.binding_stored("sub_once"));
    assert_eq!(
        stack.trigger_records("sub_once"),
        vec!["e_trigfired_sub_once_1".to_string()],
        "exactly one outcome: the delivery"
    );
}

/// A binding pass (boot, worker change) that finds a record its delivery
/// already consumed — mid-retirement, or left by a claimer that died —
/// cleans it up without reporting an expiry it never had.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pass_cleans_up_a_consumed_binding_without_an_outcome() {
    let stack = Stack::new().await;
    let mut consumed = call_binding("sub_consumed", true, AgentMessage::now_ms() - 1);
    consumed.fires = 1;
    stack.insert_binding(&consumed);

    assert!(harness::bindings::expiry::retire_due(&stack.deps, "sub_consumed", 0).await);
    assert!(!stack.binding_stored("sub_consumed"));
    assert!(
        stack.trigger_records("sub_consumed").is_empty(),
        "{:?}",
        stack.trigger_records("sub_consumed")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worker_disconnect_event_redrives_an_orphaned_turn() {
    let stack = Stack::new().await;
    stack.store.lock().unwrap().put(
        "harness_turn",
        "s_orphan",
        json!({
            "session_id": "s_orphan", "turn_id": "t_orphan", "status": "running",
            "step": 3, "turn_count": 0, "depth": 0,
            "options": { "model": "fake", "max_turns": 16 },
            "created_at": 1, "updated_at": 1
        }),
    );
    harness::engine_events::register(&stack.deps);
    tokio::spawn(harness::inflight::serve(stack.deps.clone()));

    let enqueued = |store: &Store| {
        store.calls.iter().any(|(f, data, action)| {
            f == "harness::turn"
                && action["type"] == "enqueue"
                && data["session_id"] == "s_orphan"
                && data["step"] == 3
        })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !enqueued(&stack.store.lock().unwrap()),
        "nothing re-drives without an event"
    );

    // The engine fires `engine::workers-available`: the worker that held the
    // turn's step went away.
    stack
        .push
        .send(json!({
            "type": "invokefunction",
            "invocation_id": "6f1c1a52-7c1e-4b8e-9d0a-2f3c4b5a6d7e",
            "function_id": harness::engine_events::ENGINE_CHANGE_FN_ID,
            "data": { "event": "worker_disconnected", "worker_id": "w-gone" },
        }))
        .unwrap();

    stack.wait_for("the re-driven step", enqueued).await;
    stack
        .wait_for("the restarted redrive window", |store| {
            store.get("harness_turn", "s_orphan")["updated_at"]
                .as_i64()
                .is_some_and(|at| at > 1)
        })
        .await;
}
