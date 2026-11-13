//! No polling: binding deadlines and orphaned-turn recovery run on their own
//! timers and on the engine's worker change feed, never on a periodic scan.
//! Production handlers run against an isolated mock SDK transport that can
//! also push engine-side invocations (an `engine::on-workers-changed`
//! activation).
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use harness::bindings::{Binding, BindingTarget, Lifecycle, OwnerScope, SourceSpec};
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
    /// While set, the next compare-and-set finds its record deleted — a
    /// binding retired between a delivery's read and its claim.
    retire_before_claim: bool,
    /// While set, every step enqueue fails: a woken turn cannot be scheduled.
    fail_enqueue: bool,
    /// While set, a binding record cannot be deleted: a delivery's retire fails.
    fail_binding_delete: bool,
    /// Queued when a failed turn is first written: a send that enqueued
    /// between the failing turn's first drain and its terminal write.
    queue_on_failed_write: Option<(String, Value)>,
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
            if self.fail_enqueue {
                return Err("queue unavailable".into());
            }
            return Ok(json!({ "message_receipt_id": "receipt" }));
        }
        let scope = data["scope"].as_str().unwrap_or_default();
        let key = data["key"].as_str().unwrap_or_default();
        match function {
            "state::get" | "harness::state::get" => Ok(self.get(scope, key)),
            "state::set" | "harness::state::set" => {
                self.put(scope, key, data["value"].clone());
                if scope == "harness_turn" && data["value"]["status"] == "failed" {
                    if let Some((key, row)) = self.queue_on_failed_write.take() {
                        self.put("harness_queue", &key, row);
                    }
                }
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
                if self.fail_binding_delete && scope == "harness_binding" && data["value"].is_null()
                {
                    return Err("store unavailable".into());
                }
                if std::mem::take(&mut self.retire_before_claim) {
                    self.put(scope, key, Value::Null);
                }
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
            "engine::observers::dispose" => Ok(json!({ "disposed": true })),
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
    /// Engine -> worker frames (an observer activation).
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

    fn fired_records(&self, binding: &str) -> Vec<String> {
        self.store
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(f, data, _)| {
                f == "session::append" && data["custom"]["custom_type"] == "observer_fired"
            })
            .filter(|(_, data, _)| data["custom"]["data"]["observer_id"] == binding)
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
        observer_id: Some(id.into()),
        source: Some(SourceSpec {
            function_id: "state::on-changed".into(),
            namespace: None,
            args: None,
            filters: Some(json!({
                "input.scope": { "$eq": "watched" },
                "input.key": { "$eq": "k" },
            })),
        }),
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
        dedup_key: Some(json!({ "function_id": "state::on-changed" })),
        fires: 0,
        claimed_at: None,
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
            store.appended("e_obsexpired_sub_deadline")
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

    let result = harness::functions::observer_deliver::handle(
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
        .appended("e_obsexpired_sub_fired"));
}

/// The e2e regression: a once Compose wake's terminal event reached the
/// harness twice — the binding's own observer and a recovery replay (the
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
            harness::functions::observer_deliver::handle(
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

    let replay = harness::functions::observer_deliver::handle(
        &stack.deps,
        event,
        Some(json!({ "__binding": "sub_once" })),
    )
    .await
    .unwrap();
    assert!(!replay.delivered, "{replay:?}");
    assert!(
        stack.fired_records("sub_once").is_empty(),
        "the duplicate wrote an outcome: {:?}",
        stack.fired_records("sub_once")
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
        stack.fired_records("sub_once"),
        vec!["e_obsfired_sub_once_1".to_string()],
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
        stack.fired_records("sub_consumed").is_empty(),
        "{:?}",
        stack.fired_records("sub_consumed")
    );
}

/// A once wake delivered back into its owner session.
fn wake_binding(id: &str) -> Binding {
    let mut wake = call_binding(id, true, AgentMessage::now_ms() + 60_000);
    wake.target = BindingTarget::new(harness::functions::SEND_ID);
    wake.capability = None;
    wake
}

fn park(stack: &Stack, wake: &Binding) {
    stack.insert_binding(wake);
    stack.store.lock().unwrap().put(
        "harness_turn",
        "s_owner",
        json!({
            "session_id": "s_owner", "turn_id": "t_parked", "status": "completed",
            "step": 1, "turn_count": 1, "depth": 0,
            "options": { "model": "fake", "max_turns": 16 },
            "created_at": 1, "updated_at": 1
        }),
    );
}

async fn status(stack: &Stack) -> harness::functions::status::StatusReport {
    let req = harness::functions::status::StatusRequest {
        session_id: "s_owner".into(),
        verbose: false,
    };
    harness::functions::status::handle(&stack.deps, req)
        .await
        .unwrap()
        .unwrap()
}

/// The live regression: a wake fired while its session's lock was held (a
/// running tool call). The claimed record was then deleted by the binding
/// pass every engine worker change runs, so `harness::status` read the
/// parked turn as the outcome for the seconds before the woken turn started.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_binding_pass_leaves_a_claimed_wake_to_its_delivery() {
    let stack = Stack::new().await;
    let wake = wake_binding("sub_wake");
    park(&stack, &wake);
    let held = stack.deps.locks.guard("s_owner").await;

    let delivery = {
        let deps = stack.deps.clone();
        tokio::spawn(async move {
            harness::functions::observer_deliver::handle(
                &deps,
                json!({ "type": "state", "scope": "watched", "key": "k", "new_value": 1 }),
                Some(json!({ "__binding": "sub_wake" })),
            )
            .await
        })
    };
    stack
        .wait_for("the claim", |store| {
            store.get("harness_binding", "sub_wake")["fires"] == 1
        })
        .await;
    assert!(stack
        .store
        .lock()
        .unwrap()
        .get("harness_binding", "sub_wake")["claimed_at"]
        .is_i64());

    harness::bindings::expiry::sweep(&stack.deps).await;
    harness::bindings::expiry::sweep(&stack.deps).await;
    let due = AgentMessage::now_ms();
    assert!(!harness::bindings::expiry::retire_due(&stack.deps, "sub_wake", due).await);
    assert!(
        stack.binding_stored("sub_wake"),
        "the delivery retires its own record"
    );
    let parked = status(&stack).await;
    assert_eq!(parked.turn_id.as_deref(), Some("t_parked"));
    assert_eq!(parked.status, harness::types::turn::TurnStatus::Completed);
    assert!(parked.expects_wake, "the parked turn is not the outcome");

    drop(held);
    let delivered = delivery.await.unwrap().unwrap();
    assert!(delivered.delivered, "{delivered:?}");
    let woken = status(&stack).await;
    assert_eq!(woken.status, harness::types::turn::TurnStatus::Running);
    assert_ne!(woken.turn_id.as_deref(), Some("t_parked"));
    assert!(!stack.binding_stored("sub_wake"));
    assert!(!stack.deps.deliveries.contains("sub_wake"));
}

/// A delivery that gives up before dispatching leaves no in-flight marker:
/// it would keep the binding pass off a record forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_delivery_that_loses_its_claim_leaves_no_marker() {
    let stack = Stack::new().await;
    stack.insert_binding(&wake_binding("sub_gone"));
    stack.store.lock().unwrap().retire_before_claim = true;

    let result = harness::functions::observer_deliver::handle(
        &stack.deps,
        json!({ "type": "state", "scope": "watched", "key": "k", "new_value": 1 }),
        Some(json!({ "__binding": "sub_gone" })),
    )
    .await
    .unwrap();

    assert_eq!(
        result.gate.as_deref(),
        Some("unknown-binding"),
        "{result:?}"
    );
    assert!(!stack.deps.deliveries.contains("sub_gone"));
}

/// A claimed wake whose claimer died (no delivery in flight here) is still
/// the binding pass's to delete.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_binding_pass_deletes_a_claimed_wake_nobody_delivers() {
    let stack = Stack::new().await;
    let mut wake = wake_binding("sub_orphan");
    wake.fires = 1;
    park(&stack, &wake);

    harness::bindings::expiry::sweep(&stack.deps).await;

    assert!(!stack.binding_stored("sub_orphan"));
}

fn stored_turn(stack: &Stack) -> Value {
    stack.store.lock().unwrap().get("harness_turn", "s_owner")
}

async fn deliver_wake(stack: &Stack) -> harness::functions::observer_deliver::DeliverResult {
    harness::functions::observer_deliver::handle(
        &stack.deps,
        json!({ "type": "state", "scope": "watched", "key": "k", "new_value": 1 }),
        Some(json!({ "__binding": "sub_wake" })),
    )
    .await
    .unwrap()
}

/// The woken turn fails inside the delivery, before `retire` deletes the
/// claimed record. It must still end the exchange: it is the wake's outcome,
/// not a turn parked on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_woken_turn_that_cannot_be_scheduled_ends_the_exchange() {
    let stack = Stack::new().await;
    park(&stack, &wake_binding("sub_wake"));
    stack.store.lock().unwrap().fail_enqueue = true;

    let result = deliver_wake(&stack).await;

    assert!(!result.delivered, "{result:?}");
    let turn = stored_turn(&stack);
    assert_eq!(turn["status"], "failed", "{turn}");
    assert_ne!(turn["turn_id"], "t_parked", "{turn}");
    assert_eq!(turn["exchange_finished"], true, "{turn}");
    assert!(!status(&stack).await.expects_wake);
    assert!(!stack.binding_stored("sub_wake"));
}

/// A delivery whose retire fails leaves its claimed record behind until the
/// binding pass. The woken turn and every later one must not wait on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_woken_turn_outlasting_a_failed_retire_ends_the_exchange() {
    let stack = Stack::new().await;
    park(&stack, &wake_binding("sub_wake"));
    stack.store.lock().unwrap().fail_binding_delete = true;

    let result = deliver_wake(&stack).await;

    assert!(result.delivered, "{result:?}");
    let lingering = stack
        .store
        .lock()
        .unwrap()
        .get("harness_binding", "sub_wake");
    assert_eq!(lingering["fires"], 1, "{lingering}");
    assert!(lingering["claimed_at"].is_i64(), "{lingering}");
    assert!(!stack.deps.deliveries.contains("sub_wake"));
    let woken = status(&stack).await;
    assert_eq!(woken.status, harness::types::turn::TurnStatus::Running);
    let turn_id = woken.turn_id.expect("the woken turn");
    assert_ne!(turn_id, "t_parked");

    harness::turn_loop::fail_turn(&stack.deps, "s_owner", &turn_id, "boom")
        .await
        .unwrap();

    let turn = stored_turn(&stack);
    assert_eq!(turn["exchange_finished"], true, "{turn}");
    assert!(!status(&stack).await.expects_wake);
    assert!(
        stack.binding_stored("sub_wake"),
        "the record outlived the seed"
    );
}

/// The race the claim guards against: a turn that started before the claim
/// still finalizes parked while its wake is being delivered.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_that_predates_the_claim_stays_parked_on_it() {
    let stack = Stack::new().await;
    let mut wake = wake_binding("sub_wake");
    wake.fires = 1;
    wake.claimed_at = Some(AgentMessage::now_ms());
    stack.insert_binding(&wake);
    stack.store.lock().unwrap().put(
        "harness_turn",
        "s_owner",
        json!({
            "session_id": "s_owner", "turn_id": "t_running", "status": "running",
            "step": 1, "turn_count": 1, "depth": 0,
            "options": { "model": "fake", "max_turns": 16 },
            "created_at": 1, "updated_at": 1
        }),
    );

    harness::turn_loop::fail_turn(&stack.deps, "s_owner", "t_running", "boom")
        .await
        .unwrap();

    let turn = stored_turn(&stack);
    assert_eq!(turn["status"], "failed", "{turn}");
    assert!(turn.get("exchange_finished").is_none(), "{turn}");
    assert!(status(&stack).await.expects_wake);
}

/// A message enqueued while a turn fails, after its first drain, is collected
/// by the sweep after the terminal write. The failed turn is then progress: it
/// is rewritten as continuing, not as the exchange's end, and the message
/// reseeds the session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_message_queued_as_a_turn_fails_keeps_the_exchange_going() {
    let stack = Stack::new().await;
    let row = json!({
        "id": "q_late", "session_id": "s_owner", "entry_id": "e_late", "queued_at": 1,
        "message": AgentMessage::user_text("[notification] late"),
    });
    {
        let mut store = stack.store.lock().unwrap();
        store.queue_on_failed_write = Some(("s_owner:q_late".into(), row));
        store.put(
            "harness_turn",
            "s_owner",
            json!({
                "session_id": "s_owner", "turn_id": "t_running", "status": "running",
                "step": 1, "turn_count": 1, "depth": 0,
                "options": { "model": "fake", "max_turns": 16 },
                "created_at": 1, "updated_at": 1
            }),
        );
    }

    harness::turn_loop::fail_turn(&stack.deps, "s_owner", "t_running", "boom")
        .await
        .unwrap();

    let store = stack.store.lock().unwrap();
    let failed = store
        .calls
        .iter()
        .filter(|(f, data, _)| {
            f.ends_with("state::set")
                && data["scope"] == "harness_turn"
                && data["value"]["turn_id"] == "t_running"
        })
        .map(|(_, data, _)| &data["value"])
        .next_back()
        .expect("the failed turn is written");
    assert_eq!(failed["status"], "failed", "{failed}");
    assert_eq!(failed["continues"], true, "{failed}");
    assert!(failed.get("exchange_finished").is_none(), "{failed}");
    assert!(store.appended("e_late"));
    let turn = store.get("harness_turn", "s_owner");
    assert_eq!(turn["status"], "running", "the message reseeds: {turn}");
    assert_ne!(turn["turn_id"], "t_running", "{turn}");
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

    // The engine calls `engine::on-workers-changed` and activates the
    // harness's observer with the `Observed` envelope, the event as its
    // output (`capture: [output]`): the worker that held the turn's step
    // went away.
    stack
        .push
        .send(json!({
            "type": "invokefunction",
            "invocation_id": "6f1c1a52-7c1e-4b8e-9d0a-2f3c4b5a6d7e",
            "function_id": harness::engine_events::ENGINE_CHANGE_FN_ID,
            "data": {
                "observer_id": "engine-change",
                "activation_id": "act-1",
                "function_id": "engine::on-workers-changed",
                "invocation_id": "inv-1",
                "status": "success",
                "started_at_ms": 1,
                "duration_ms": 0.5,
                "output": { "event": "worker_disconnected", "worker_id": "w-gone" },
            },
            "activation": { "observer_id": "engine-change", "activation_id": "act-1" },
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
