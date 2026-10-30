//! The harness's observer surfaces against a real engine (`III_ENGINE_BIN`,
//! or `iii` on PATH; skipped without one, failing when `III_ENGINE_BIN` names
//! one that does not start):
//!
//! - `harness::observer::deliver` fires a binding from the `Observed`
//!   envelope of iii's delivery and from the raw payload a source's deliver
//!   hook sends through the observer.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use harness::bindings::{Binding, BindingTarget, Lifecycle, OwnerScope, SourceSpec};
use harness::config::WorkerConfig;
use harness::deps::Deps;
use harness::events::TurnEvents;
use harness::types::turn::FunctionPolicy;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, Call, CallAction, CallRequest, IIIClient, InitOptions, Observer,
    RegisterFunction,
};
use serde_json::{json, Value};
use tokio::sync::RwLock;

struct Engine {
    url: String,
    child: std::process::Child,
    _dir: tempfile::TempDir,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn worker_options(name: &str) -> InitOptions {
    InitOptions {
        metadata: Some(WorkerMetadata {
            name: name.to_string(),
            ..WorkerMetadata::default()
        }),
        ..InitOptions::default()
    }
}

async fn isolated_engine() -> Option<Engine> {
    let binary = std::env::var_os("III_ENGINE_BIN")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::process::Command::new("iii")
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .ok()
                .filter(|status| status.success())
                .map(|_| std::path::PathBuf::from("iii"))
        })?;
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()?
        .port();
    let dir = tempfile::tempdir().ok()?;
    let config = dir.path().join("config.yaml");
    std::fs::write(
        &config,
        format!("workers:\n  - name: iii-worker-manager\n    config:\n      port: {port}\n"),
    )
    .ok()?;
    let mut child = std::process::Command::new(binary)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&config)
        .current_dir(dir.path())
        .env("III_TELEMETRY_ENABLED", "false")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let url = format!("ws://127.0.0.1:{port}");
    let probe = register_worker(&url, worker_options("probe"));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if probe
            .call(CallRequest {
                function_id: "engine::workers::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(500),
            })
            .await
            .is_ok()
        {
            probe.shutdown_async().await;
            return Some(Engine {
                url,
                child,
                _dir: dir,
            });
        }
        if Instant::now() >= deadline {
            probe.shutdown_async().await;
            let _ = child.kill();
            let _ = child.wait();
            if std::env::var_os("III_ENGINE_BIN").is_some() {
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                panic!("iii engine did not become ready: {stderr}");
            }
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

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
                    async move { Ok::<_, iii_sdk::Error>(stack.respond(&id, &data)) }
                }),
            );
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
    source.on_observer_deliver_for(
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
