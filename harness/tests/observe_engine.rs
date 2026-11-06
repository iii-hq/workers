//! The harness's observer surfaces against a real engine (`III_ENGINE_BIN`,
//! or `iii` on PATH; skipped without one, failing when `III_ENGINE_BIN` names
//! one that does not start):
//!
//! - `harness::on-ready` sends an observer added after boot the ready event as
//!   a call of the function for that observer alone, delivered in the shape
//!   its `deliver` asks for, and no other observer receives it;
//! - `harness::observer::deliver` fires a binding from the `Observed`
//!   envelope of iii's delivery and from the raw payload a source's deliver
//!   hook sends through the observer;
//! - a harness back on a new connection under the same worker name replaces
//!   its binding's suspended observer by observing the same id again;
//! - a wake that fails to deliver is retried, then left armed, never retired
//!   undelivered; when it cannot stay armed its owner is told;
//! - removing a wake a fire just claimed does not report it lost;
//! - the boot sweep retires bindings on sources agents may not observe.

#[path = "support/engine.rs"]
mod engine;

use engine::{isolated_engine, worker_options, Engine};

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use harness::bindings::{Binding, BindingTarget, Lifecycle, OwnerScope, SourceSpec};
use harness::config::WorkerConfig;
use harness::deps::Deps;
use harness::events::{TurnEvents, ON_READY_FN};
use harness::functions::observer_deliver::DELIVER_ID;
use harness::types::turn::FunctionPolicy;
use iii_sdk::{
    register_worker, Call, CallAction, CallRequest, Deliver, IIIClient, Observer, RegisterFunction,
    Source,
};
use serde_json::{json, Value};
use tokio::sync::RwLock;

/// Function ids the stub stack serves to the delivery hop: the harness's
/// state, the owner's session, the approval gate, and the binding's target.
const STACK_FUNCTIONS: [&str; 15] = [
    "state::get",
    "state::set",
    "state::delete",
    "state::list",
    "state::list_keys",
    "state::claim-namespace",
    "harness::state::get",
    "harness::state::set",
    "harness::state::delete",
    "harness::state::list",
    "harness::state::compare-and-set",
    "session::get",
    "session::append",
    "approval::evaluate",
    "sink::record",
];

/// An in-memory stand-in for the state worker, session-manager, the approval
/// gate and a binding target.
#[derive(Clone, Default)]
struct Stack {
    state: Arc<Mutex<BTreeMap<(String, String), Value>>>,
    sink: Arc<Mutex<Vec<Value>>>,
    appended: Arc<Mutex<Vec<Value>>>,
    /// How many more calls of a function fail before it answers again.
    failures: Arc<Mutex<BTreeMap<String, usize>>>,
    /// A binding that one fire claims right before the next compare-and-set
    /// deleting it: a fire landing between a remover's read and its delete.
    fire_before_delete: Arc<Mutex<Option<String>>>,
    /// A binding whose record is deleted by the next failing call: a sweep
    /// cleaning up a claimed record while its delivery is failing.
    delete_on_failure: Arc<Mutex<Option<String>>>,
}

impl Stack {
    fn get(&self, scope: &str, key: &str) -> Value {
        self.state
            .lock()
            .unwrap()
            .get(&(scope.into(), key.into()))
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn put(&self, scope: &str, key: &str, value: Value) {
        let mut state = self.state.lock().unwrap();
        if value.is_null() {
            state.remove(&(scope.into(), key.into()));
        } else {
            state.insert((scope.into(), key.into()), value);
        }
    }

    fn respond(&self, function: &str, data: &Value) -> Value {
        let scope = data["scope"].as_str().unwrap_or_default();
        let key = data["key"].as_str().unwrap_or_default();
        match function {
            "state::get" | "harness::state::get" => self.get(scope, key),
            "state::set" | "harness::state::set" => {
                self.put(scope, key, data["value"].clone());
                json!({})
            }
            "state::delete" | "harness::state::delete" => {
                self.put(scope, key, Value::Null);
                json!({})
            }
            "state::list_keys" => json!({ "keys": self
                .state
                .lock()
                .unwrap()
                .keys()
                .filter(|(s, _)| s == scope)
                .map(|(_, k)| k.clone())
                .collect::<Vec<_>>() }),
            "state::list" | "harness::state::list" => Value::Array(
                self.state
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|((s, _), _)| s == scope)
                    .map(|(_, v)| v.clone())
                    .collect(),
            ),
            "harness::state::compare-and-set" => {
                let mut armed = self.fire_before_delete.lock().unwrap();
                if scope == "harness_binding"
                    && data["value"].is_null()
                    && armed.as_deref() == Some(key)
                {
                    *armed = None;
                    let mut record = self.get(scope, key);
                    record["fires"] = json!(record["fires"].as_u64().unwrap_or(0) + 1);
                    self.put(scope, key, record);
                }
                drop(armed);
                let current = self.get(scope, key);
                let swapped = current == data["expected"];
                if swapped {
                    self.put(scope, key, data["value"].clone());
                }
                json!({ "swapped": swapped, "current": current })
            }
            "state::claim-namespace" => json!({ "claimed": true }),
            "session::get" => json!({ "meta": {
                "session_id": data["session_id"], "title": "owner", "metadata": {}
            }}),
            "session::append" => {
                self.appended.lock().unwrap().push(data.clone());
                json!({ "entry_id": data["entry_id"] })
            }
            "approval::evaluate" => json!({ "verdict": "allow" }),
            "sink::record" => {
                self.sink.lock().unwrap().push(data.clone());
                json!({ "recorded": true })
            }
            _ => Value::Null,
        }
    }

    fn register(&self, iii: &IIIClient) {
        for function_id in STACK_FUNCTIONS {
            let stack = self.clone();
            let id = function_id.to_string();
            iii.register_function(
                function_id,
                RegisterFunction::new_async(move |data: Value| {
                    let stack = stack.clone();
                    let id = id.clone();
                    async move {
                        if stack.failing(&id, &data) {
                            if let Some(binding) = stack.delete_on_failure.lock().unwrap().take() {
                                stack.put("harness_binding", &binding, Value::Null);
                            }
                            return Err(iii_sdk::Error::Remote {
                                code: "UNAVAILABLE".into(),
                                message: format!("{id} is down"),
                                stacktrace: None,
                            });
                        }
                        Ok::<_, iii_sdk::Error>(stack.respond(&id, &data))
                    }
                }),
            );
        }
    }

    /// Fail the next `calls` calls of `call`: a function id, or a function
    /// id and the state scope it acts on (`"state::set harness_queue"`).
    fn fail(&self, call: &str, calls: usize) {
        self.failures.lock().unwrap().insert(call.into(), calls);
    }

    fn failing(&self, function_id: &str, data: &Value) -> bool {
        let scoped = format!(
            "{function_id} {}",
            data["scope"].as_str().unwrap_or_default()
        );
        let mut failures = self.failures.lock().unwrap();
        for call in [scoped.as_str(), function_id] {
            if let Some(left) = failures.get_mut(call).filter(|left| **left > 0) {
                *left -= 1;
                return true;
            }
        }
        false
    }

    /// Give `s_owner` a running turn, as a session parked on its wakes has
    /// between steps: a wake delivered to it is queued for that turn.
    fn park_owner(&self) {
        self.put(
            "harness_turn",
            "s_owner",
            json!({
                "session_id": "s_owner", "turn_id": "t_owner", "status": "running",
                "step": 1, "turn_count": 1, "depth": 0,
                "options": {"model": "fake", "max_turns": 16},
                "created_at": 1, "updated_at": 1
            }),
        );
    }

    /// The messages queued for a turn: the wakes delivered to it.
    fn queued(&self) -> Vec<Value> {
        self.state
            .lock()
            .unwrap()
            .iter()
            .filter(|((scope, _), _)| scope == "harness_queue")
            .map(|(_, row)| row.clone())
            .collect()
    }

    async fn wait_until(&self, what: &str, done: impl Fn(&Stack) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(self) {
            assert!(
                Instant::now() < deadline,
                "{what}: timed out; appended: {:?}",
                self.appended.lock().unwrap()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn insert_binding(&self, binding: &Binding) {
        self.put(
            "harness_binding",
            &binding.id,
            serde_json::to_value(binding).unwrap(),
        );
    }

    fn sink(&self) -> Vec<Value> {
        self.sink.lock().unwrap().clone()
    }

    async fn wait_sink(&self, count: usize) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let sink = self.sink();
            if sink.len() >= count {
                return sink;
            }
            assert!(
                Instant::now() < deadline,
                "sink::record received {} of {count}: {sink:?}; appended: {:?}",
                sink.len(),
                self.appended.lock().unwrap()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn fired(&self, binding: &str) -> Vec<Value> {
        self.appended
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| entry["custom"]["custom_type"] == "observer_fired")
            .filter(|entry| entry["custom"]["data"]["observer_id"] == binding)
            .cloned()
            .collect()
    }
}

/// A binding of `function_id` whose fires call `sink::record` with the event
/// at `/event`, and that never retires.
fn sink_binding(id: &str, function_id: &str) -> Binding {
    Binding {
        id: id.into(),
        observer_id: Some(id.into()),
        source: Some(SourceSpec {
            function_id: function_id.into(),
            namespace: None,
            args: None,
            filters: None,
        }),
        owner: OwnerScope {
            session_id: "s_owner".into(),
            root_session_id: None,
        },
        target: BindingTarget::new("sink::record"),
        conditions: vec![],
        lifecycle: Lifecycle {
            once: false,
            max_fires: None,
            expires_at: None,
        },
        capability: Some(FunctionPolicy {
            allow: vec!["sink::*".into()],
            deny: vec![],
            expose: Default::default(),
        }),
        dedup_key: Some(json!({ "function_id": function_id })),
        fires: 0,
        created_at: 0,
    }
}

/// A one-shot wake of `s_owner` on `function_id`: what a parked session
/// waits on.
fn wake_binding(id: &str, function_id: &str) -> Binding {
    let mut binding = sink_binding(id, function_id);
    binding.target = BindingTarget::new(harness::functions::SEND_ID);
    binding.lifecycle.once = true;
    binding.capability = None;
    binding
}

/// The stub stack, registered on `engine`.
async fn stack_on(engine: &Engine) -> (Stack, IIIClient) {
    let stack = Stack::default();
    let stub = register_worker(&engine.url, worker_options("stack"));
    stack.register(&stub);
    stub.wait_until_registered(Duration::from_secs(5))
        .await
        .expect("stack registered");
    (stack, stub)
}

/// Store `binding` and observe its source.
async fn arm(deps: &Deps, stack: &Stack, binding: &Binding) {
    stack.insert_binding(binding);
    let handle = harness::bindings::observe_binding(deps, binding).expect("a source");
    tokio::time::timeout(Duration::from_secs(10), handle.ready())
        .await
        .expect("observer settles")
        .unwrap_or_else(|error| panic!("{}: {error}", binding.id));
}

/// A harness connection under the worker name `harness` with the delivery
/// hop and every other harness function registered.
async fn harness_connection(engine: &Engine) -> Arc<Deps> {
    let iii = Arc::new(register_worker(&engine.url, worker_options("harness")));
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
        TurnEvents::register(&iii),
        harness::hooks::HookRegistry::register(&iii),
    ));
    harness::functions::register_all(&iii, &deps);
    iii.wait_until_registered(Duration::from_secs(10))
        .await
        .expect("harness functions registered");
    deps
}

/// The event source of the tests: `demo::on-event` is delivered by iii;
/// `kanban::on-change` delivers itself, as kanban does, calling through each
/// observer whose filters match with its own payload.
async fn event_sources(engine: &Engine) -> IIIClient {
    let source = register_worker(&engine.url, worker_options("event-source"));
    source.register_function(
        "demo::on-event",
        RegisterFunction::new(|event: Value| Ok::<Value, iii_sdk::Error>(event)).private(),
    );
    let client = source.clone();
    source.on_observation_deliver_for(
        "kanban::on-change",
        move |call: Call, observers: Vec<Observer>| {
            let iii = client.clone();
            async move {
                for obs in observers {
                    if !obs.matches(&call) {
                        continue;
                    }
                    let payload = json!({ "summary": "T-1 moved to done", "change": call.input });
                    let request = CallRequest::observer(&obs, payload).action(CallAction::Void);
                    if let Err(error) = iii.call(request).await {
                        panic!("call through {} failed: {error}", obs.id());
                    }
                }
                Ok::<Value, iii_sdk::Error>(call.input)
            }
        },
    );
    source.register_function(
        "kanban::on-change",
        RegisterFunction::new(|event: Value| Ok::<Value, iii_sdk::Error>(event)).private(),
    );
    source
        .wait_until_registered(Duration::from_secs(5))
        .await
        .expect("event sources registered");
    source
}

async fn emit(source: &IIIClient, function_id: &str, event: Value) {
    source
        .call(CallRequest {
            function_id: function_id.into(),
            payload: event,
            action: Some(CallAction::Void),
            timeout_ms: None,
        })
        .await
        .unwrap_or_else(|error| panic!("{function_id}: {error}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_binding_fires_from_an_envelope_and_from_a_deliver_hook_payload() {
    let Some(engine) = isolated_engine().await else {
        eprintln!("skipping: iii engine is unavailable");
        return;
    };
    let stack = Stack::default();
    let stub = register_worker(&engine.url, worker_options("stack"));
    stack.register(&stub);
    stub.wait_until_registered(Duration::from_secs(5))
        .await
        .expect("stack registered");
    let source = event_sources(&engine).await;
    let deps = harness_connection(&engine).await;

    let enveloped = sink_binding("sub_envelope", "demo::on-event");
    let hooked = sink_binding("sub_hooked", "kanban::on-change");
    for binding in [&enveloped, &hooked] {
        stack.insert_binding(binding);
        let handle = harness::bindings::observe_binding(&deps, binding).expect("a source");
        tokio::time::timeout(Duration::from_secs(10), handle.ready())
            .await
            .expect("observer settles")
            .unwrap_or_else(|error| panic!("{}: {error}", binding.id));
    }

    // iii's delivery: the Observed envelope; the event is the call's output.
    emit(
        &source,
        "demo::on-event",
        json!({ "scope": "run", "key": "go" }),
    )
    .await;
    let sink = stack.wait_sink(1).await;
    assert_eq!(
        sink[0]["event"],
        json!({ "scope": "run", "key": "go" }),
        "{sink:?}"
    );

    // A deliver hook's call through the observer: the payload as sent.
    emit(
        &source,
        "kanban::on-change",
        json!({ "ticket_id": "T-1", "to": "done" }),
    )
    .await;
    let sink = stack.wait_sink(2).await;
    assert_eq!(
        sink[1]["event"],
        json!({ "summary": "T-1 moved to done", "change": { "ticket_id": "T-1", "to": "done" } }),
        "{sink:?}"
    );

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(stack.sink().len(), 2, "one fire each: {:?}", stack.sink());
    for binding in ["sub_envelope", "sub_hooked"] {
        let fired = stack.fired(binding);
        assert_eq!(fired.len(), 1, "{binding}: {fired:?}");
        assert_eq!(
            fired[0]["custom"]["data"]["outcome"],
            json!("delivered"),
            "{fired:?}"
        );
    }
    let stored = stack.get("harness_binding", "sub_hooked");
    assert_eq!(stored["fires"], json!(1), "{stored}");

    source.shutdown_async().await;
    deps.iii.shutdown_async().await;
    stub.shutdown_async().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restarted_harness_replaces_its_suspended_binding_observer() {
    let Some(engine) = isolated_engine().await else {
        eprintln!("skipping: iii engine is unavailable");
        return;
    };
    let stack = Stack::default();
    let stub = register_worker(&engine.url, worker_options("stack"));
    stack.register(&stub);
    stub.wait_until_registered(Duration::from_secs(5))
        .await
        .expect("stack registered");
    let source = event_sources(&engine).await;
    let binding = sink_binding("sub_stable", "demo::on-event");
    stack.insert_binding(&binding);

    let first = harness_connection(&engine).await;
    let handle = harness::bindings::observe_binding(&first, &binding).expect("a source");
    handle.ready().await.expect("first observer active");
    // The connection goes away without disposing anything: the engine
    // suspends the observer for its grace period.
    first.iii.shutdown_async().await;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let listed = source
            .call(CallRequest::new("engine::observers::list", json!({})))
            .await
            .expect("engine::observers::list");
        let state = listed["observers"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|observer| observer["id"] == "sub_stable")
            .map(|observer| observer["state"].clone());
        if state == Some(json!("suspended")) {
            break;
        }
        assert!(Instant::now() < deadline, "never suspended: {listed}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // The harness back on a new connection observes the same id, under the
    // same worker name: the engine replaces the suspended observer with it.
    let second = harness_connection(&engine).await;
    let handle = harness::bindings::observe_binding(&second, &binding).expect("a source");
    tokio::time::timeout(Duration::from_secs(10), handle.ready())
        .await
        .expect("observer settles")
        .expect("the same id is active again");
    assert_eq!(handle.id(), "sub_stable");

    emit(&source, "demo::on-event", json!({ "n": 1 })).await;
    let sink = stack.wait_sink(1).await;
    assert_eq!(sink[0]["event"], json!({ "n": 1 }), "{sink:?}");
    let listed = source
        .call(CallRequest::new("engine::observers::list", json!({})))
        .await
        .expect("engine::observers::list");
    let held: Vec<&Value> = listed["observers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|observer| observer["id"] == "sub_stable")
        .collect();
    assert_eq!(held.len(), 1, "{listed}");
    assert_eq!(held[0]["state"], json!("active"), "{listed}");
    assert_eq!(
        held[0]["target"]["function_id"],
        json!(DELIVER_ID),
        "{listed}"
    );

    source.shutdown_async().await;
    second.iii.shutdown_async().await;
    stub.shutdown_async().await;
}

/// Payloads each test target function received, by function id.
#[derive(Clone, Default)]
struct Received(Arc<Mutex<BTreeMap<String, Vec<Value>>>>);

impl Received {
    /// Register `function_id` on `iii`, recording every payload it gets.
    fn target(&self, iii: &IIIClient, function_id: &str) {
        let received = self.clone();
        let id = function_id.to_string();
        iii.register_function(
            function_id,
            RegisterFunction::new_async(move |payload: Value| {
                let received = received.clone();
                let id = id.clone();
                async move {
                    received
                        .0
                        .lock()
                        .unwrap()
                        .entry(id)
                        .or_default()
                        .push(payload);
                    Ok::<_, iii_sdk::Error>(json!({}))
                }
            }),
        );
    }

    fn of(&self, function_id: &str) -> Vec<Value> {
        self.0
            .lock()
            .unwrap()
            .get(function_id)
            .cloned()
            .unwrap_or_default()
    }

    async fn wait(&self, function_id: &str, count: usize) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let got = self.of(function_id);
            if got.len() >= count {
                return got;
            }
            assert!(
                Instant::now() < deadline,
                "{function_id} received {} of {count}: {got:?}",
                got.len()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default();
    keys.retain(|key| key != "metadata");
    keys.sort();
    keys
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_late_on_ready_observer_receives_the_ready_event_alone_in_its_own_shape() {
    let Some(engine) = isolated_engine().await else {
        eprintln!("skipping: iii engine is unavailable");
        return;
    };
    let host = Arc::new(register_worker(&engine.url, worker_options("harness")));
    let events = TurnEvents::register(&host);
    host.wait_until_registered(Duration::from_secs(5))
        .await
        .expect("harness events registered");

    let consumer = register_worker(&engine.url, worker_options("ready-consumer"));
    let received = Received::default();
    for target in [
        "test::boot-envelope",
        "test::boot-output",
        "test::late-envelope",
        "test::late-output",
    ] {
        received.target(&consumer, target);
    }
    consumer
        .wait_until_registered(Duration::from_secs(5))
        .await
        .expect("targets registered");

    // Observers present at boot receive the boot ready event.
    let boot_envelope = consumer.observe(Source::function(ON_READY_FN), "test::boot-envelope");
    let boot_output = consumer.observe(
        Source::function(ON_READY_FN).deliver(Deliver::Output),
        "test::boot-output",
    );
    boot_envelope.ready().await.expect("boot envelope observer");
    boot_output.ready().await.expect("boot output observer");
    events.emit_ready().await;
    let boot_env_event = received.wait("test::boot-envelope", 1).await.remove(0);
    let boot_out_event = received.wait("test::boot-output", 1).await.remove(0);

    // Observers added after boot receive the current ready event once each.
    let late_envelope = consumer.observe(Source::function(ON_READY_FN), "test::late-envelope");
    let late_output = consumer.observe(
        Source::function(ON_READY_FN).deliver(Deliver::Output),
        "test::late-output",
    );
    late_envelope.ready().await.expect("late envelope observer");
    late_output.ready().await.expect("late output observer");
    let late_env_event = received.wait("test::late-envelope", 1).await.remove(0);
    let late_out_event = received.wait("test::late-output", 1).await.remove(0);
    tokio::time::sleep(Duration::from_millis(500)).await;

    for target in [
        "test::boot-envelope",
        "test::boot-output",
        "test::late-envelope",
        "test::late-output",
    ] {
        assert_eq!(
            received.of(target).len(),
            1,
            "{target} receives one ready event: {:?}",
            received.of(target)
        );
    }

    // The default envelope: the ready call iii delivered, for that observer.
    assert_eq!(
        late_env_event["function_id"],
        json!(ON_READY_FN),
        "{late_env_event}"
    );
    assert_eq!(
        late_env_event["observer_id"],
        json!(late_envelope.id()),
        "{late_env_event}"
    );
    assert_eq!(
        late_env_event["output"]["status"],
        json!("ready"),
        "{late_env_event}"
    );
    assert_eq!(
        boot_env_event["observer_id"],
        json!(boot_envelope.id()),
        "{boot_env_event}"
    );
    assert_eq!(
        keys(&late_env_event),
        keys(&boot_env_event),
        "same envelope shape"
    );

    // `deliver: output`: the ready event itself, as the boot event arrived.
    assert_eq!(late_out_event["status"], json!("ready"), "{late_out_event}");
    assert!(late_out_event["timestamp"].is_i64(), "{late_out_event}");
    assert_eq!(
        keys(&late_out_event),
        keys(&boot_out_event),
        "same event shape"
    );
    assert_eq!(keys(&late_out_event), vec!["status", "timestamp"]);

    consumer.shutdown_async().await;
    host.shutdown_async().await;
}

/// The write that queues a wake for a running turn, and nothing else on the
/// delivery path.
const QUEUE_WRITE: &str = "state::set harness_queue";

/// A wake whose delivery hits a failing dependency is retried; one that keeps
/// failing gives its fire back and stays armed. Retired undelivered, it left
/// the parked owner waiting on an event it would never hear.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_undelivered_wake_is_retried_then_left_armed() {
    let Some(engine) = isolated_engine().await else {
        eprintln!("skipping: iii engine is unavailable");
        return;
    };
    let (stack, stub) = stack_on(&engine).await;
    let source = event_sources(&engine).await;
    let deps = harness_connection(&engine).await;
    stack.park_owner();

    // One failed write of the turn's queue: the second attempt delivers.
    arm(
        &deps,
        &stack,
        &wake_binding("sub_retried", "demo::on-event"),
    )
    .await;
    stack.fail(QUEUE_WRITE, 1);
    emit(&source, "demo::on-event", json!({ "n": 1 })).await;
    stack
        .wait_until("the retried wake reaches the turn", |s| {
            s.queued().len() == 1
        })
        .await;
    stack
        .wait_until("its fire is recorded", |s| {
            !s.fired("sub_retried").is_empty()
        })
        .await;
    let fired = stack.fired("sub_retried");
    assert_eq!(
        fired[0]["custom"]["data"]["outcome"],
        json!("delivered"),
        "{fired:?}"
    );
    assert!(stack.get("harness_binding", "sub_retried").is_null());

    // A queue that stays down: the fire is given back and the wake stays armed.
    arm(&deps, &stack, &wake_binding("sub_armed", "demo::on-event")).await;
    stack.fail(QUEUE_WRITE, usize::MAX);
    emit(&source, "demo::on-event", json!({ "n": 2 })).await;
    stack
        .wait_until("the failed fire is recorded", |s| {
            s.fired("sub_armed").len() == 1
        })
        .await;
    let failed = stack.fired("sub_armed")[0]["custom"].clone();
    assert_eq!(
        failed["data"]["outcome"],
        json!("delivery_failed"),
        "{failed}"
    );
    assert_eq!(failed["data"]["retired"], json!(false), "{failed}");
    let stored = stack.get("harness_binding", "sub_armed");
    assert_eq!(
        stored["fires"],
        json!(0),
        "the fire was not given back: {stored}"
    );

    // The next event delivers it, under a record of its own.
    stack.fail(QUEUE_WRITE, 0);
    emit(&source, "demo::on-event", json!({ "n": 3 })).await;
    stack
        .wait_until("the armed wake reaches the turn", |s| s.queued().len() == 2)
        .await;
    stack
        .wait_until("its fire is recorded", |s| s.fired("sub_armed").len() == 2)
        .await;
    let fired = stack.fired("sub_armed");
    assert_eq!(
        fired[1]["custom"]["data"]["outcome"],
        json!("delivered"),
        "{fired:?}"
    );
    assert_ne!(fired[0]["entry_id"], fired[1]["entry_id"], "{fired:?}");
    assert!(stack.get("harness_binding", "sub_armed").is_null());

    source.shutdown_async().await;
    deps.iii.shutdown_async().await;
    stub.shutdown_async().await;
}

/// A wake that a fire claims between the removal's read and its delete was
/// delivered: the owner must not also be told it was removed before firing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disposing_a_wake_a_fire_just_claimed_reports_no_lost_wake() {
    let Some(engine) = isolated_engine().await else {
        eprintln!("skipping: iii engine is unavailable");
        return;
    };
    let (stack, stub) = stack_on(&engine).await;
    let deps = harness_connection(&engine).await;
    stack.insert_binding(&wake_binding("sub_raced", "demo::on-event"));
    *stack.fire_before_delete.lock().unwrap() = Some("sub_raced".into());

    let request = serde_json::from_value(json!({
        "observer_id": "sub_raced", "session_id": "s_owner"
    }))
    .unwrap();
    let response = harness::functions::observers_list::dispose(&deps, request)
        .await
        .expect("disposed");
    assert!(response.removed);
    assert!(stack.get("harness_binding", "sub_raced").is_null());
    let records = stack.fired("sub_raced");
    assert_eq!(records.len(), 1, "{records:?}");
    let note = records[0]["custom"]["data"]["note"]
        .as_str()
        .unwrap_or_default();
    assert!(!note.contains("before any fire"), "{records:?}");

    deps.iii.shutdown_async().await;
    stub.shutdown_async().await;
}

/// A binding on a harness function agents may not observe, from a build that
/// allowed it, is retired by the boot sweep and its owner told; before, each
/// boot re-armed it forever. A binding on an ordinary source is kept.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_boot_sweep_retires_a_binding_on_a_refused_source() {
    let Some(engine) = isolated_engine().await else {
        eprintln!("skipping: iii engine is unavailable");
        return;
    };
    let (stack, stub) = stack_on(&engine).await;
    let deps = harness_connection(&engine).await;
    let mut refused = wake_binding("sub_refused", "harness::on-turn-completed");
    refused.lifecycle.once = false;
    stack.insert_binding(&refused);
    stack.insert_binding(&wake_binding("sub_kept", "demo::on-event"));

    harness::bindings::gc::run(&deps).await;

    assert!(stack.get("harness_binding", "sub_refused").is_null());
    assert!(!stack.get("harness_binding", "sub_kept").is_null());
    let records = stack.fired("sub_refused");
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        records[0]["custom"]["data"]["outcome"],
        json!("invalidated")
    );

    deps.iii.shutdown_async().await;
    stub.shutdown_async().await;
}

/// A wake cannot give its fire back once its record was retired mid-delivery
/// (a pass cleaning up the claimed record, its deadline, a removal): the
/// owner is told the wake is lost. A wake refused for good (its session has
/// no turn) is not retried or given back: it retires as before.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wake_that_cannot_be_given_back_tells_its_owner() {
    let Some(engine) = isolated_engine().await else {
        eprintln!("skipping: iii engine is unavailable");
        return;
    };
    let (stack, stub) = stack_on(&engine).await;
    let source = event_sources(&engine).await;
    let deps = harness_connection(&engine).await;
    stack.park_owner();

    arm(&deps, &stack, &wake_binding("sub_gone", "demo::on-event")).await;
    stack.fail(QUEUE_WRITE, 3);
    *stack.delete_on_failure.lock().unwrap() = Some("sub_gone".into());
    emit(&source, "demo::on-event", json!({ "n": 1 })).await;
    stack
        .wait_until("the owner is told the wake is lost", |s| {
            s.queued()
                .iter()
                .any(|row| row["entry_id"] == "e_undelivered_sub_gone")
        })
        .await;
    assert!(stack.get("harness_binding", "sub_gone").is_null());

    let mut refused = wake_binding("sub_cold", "demo::on-event");
    refused.target.payload = Some(json!({ "session_id": "s_cold" }));
    arm(&deps, &stack, &refused).await;
    emit(&source, "demo::on-event", json!({ "n": 2 })).await;
    stack
        .wait_until("the refused fire is recorded", |s| {
            !s.fired("sub_cold").is_empty()
        })
        .await;
    let fired = stack.fired("sub_cold");
    assert_eq!(
        fired[0]["custom"]["data"]["outcome"],
        json!("delivery_failed"),
        "{fired:?}"
    );
    assert!(
        stack.get("harness_binding", "sub_cold").is_null(),
        "a refused wake was given back"
    );

    source.shutdown_async().await;
    deps.iii.shutdown_async().await;
    stub.shutdown_async().await;
}
