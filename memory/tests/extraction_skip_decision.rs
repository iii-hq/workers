//! In-process SDK websocket transport: engine metadata is stamped outside data.
//! This is not a live-engine ACL test. No credentials, models or real banks.
use futures_util::{SinkExt, StreamExt};
use iii_sdk::{CallRequest, Capture, Source};
use memory::{config::WorkerConfig, deps::Deps, events, hook_binding, store::Store};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio_tungstenite::tungstenite::Message;

#[derive(Default)]
struct Recorded {
    calls: Vec<Value>,
    registrations: Vec<Value>,
    decisions: Vec<Value>,
    mode: String,
}
impl Recorded {
    fn count(&self, function: &str) -> usize {
        self.calls
            .iter()
            .filter(|call| call["function_id"] == function)
            .count()
    }
    fn counters(&self) -> BTreeMap<&str, usize> {
        [
            ("event", events::ON_EXTRACTION_DECISION_FN),
            ("session_get", "session::get"),
            ("owner_function", "engine::functions::info"),
            ("owner_worker", "engine::workers::info"),
            ("queue", "engine::queue::enqueue"),
            ("model", "router::complete"),
            ("state_write", "state::set"),
            ("item_event", events::ON_ITEM_CHANGED_FN),
            ("bank_event", events::ON_BANK_CHANGED_FN),
        ]
        .into_iter()
        .map(|(name, function)| (name, self.count(function)))
        .collect()
    }
}
struct Delivery {
    data: Value,
    caller: Value,
    activation: Value,
    answer: oneshot::Sender<Value>,
}
struct Stack {
    deps: Arc<Deps>,
    tx: mpsc::UnboundedSender<Delivery>,
    recorded: Arc<Mutex<Recorded>>,
    handle: iii_sdk::ObserverHandle,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Stack {
    fn drop(&mut self) {
        self.deps.iii.shutdown();
        self.task.abort();
    }
}
fn meta(kind: Value) -> Value {
    // Actual SessionMeta fields from session-manager/src/types.rs, not role claims.
    json!({"meta":{"session_id":"synthetic-session","title":"","description":"",
        "status":"done","kind":kind,"metadata":{},"created_at":1,"updated_at":2,"message_count":0}})
}
fn completed() -> Value {
    json!({"session_id":"synthetic-session","turn_id":"synthetic-turn",
        "status":"completed","terminal":true,"timestamp":2})
}
fn observed(output: Value) -> Value {
    json!({"observer_id":"payload-observer-is-not-authority","activation_id":"act-fixture",
        "function_id":hook_binding::COMPLETION_SOURCE,"invocation_id":"inv-fixture",
        "status":"success","started_at_ms":1,"duration_ms":0.5,"output":output})
}
fn snapshot(path: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, p: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(p).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(
                    p.strip_prefix(root).unwrap().to_string_lossy().into(),
                    std::fs::read(p).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(path, path, &mut out);
    out
}
impl Stack {
    async fn new(root: &Path) -> Self {
        Self::with_binding(root, true).await
    }
    async fn with_binding(root: &Path, install_binding: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel::<Delivery>();
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let log = recorded.clone();
        let (ready, wait) = oneshot::channel();
        let task = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            ws.send(Message::Text(json!({"type":"workerregistered","worker_id":"memory-fixture","features":["observe.v2"]}).to_string().into())).await.unwrap();
            let _ = ready.send(());
            let mut seq = 0;
            let mut pending = std::collections::HashMap::new();
            loop {
                tokio::select! {
                    delivery = rx.recv() => {
                        let Some(d) = delivery else { break; };
                        if let Some(state) = d.data["fixture_state"].as_str() {
                            ws.send(Message::Text(json!({"type":"observerstatechanged","id":d.activation["observer_id"],"state":state}).to_string().into())).await.unwrap();
                            let _ = d.answer.send(json!({"result":{"ok":true}}));
                            continue;
                        }
                        seq += 1;
                        let id = format!("00000000-0000-4000-8000-{seq:012}");
                        pending.insert(id.clone(), d.answer);
                        ws.send(Message::Text(json!({"type":"invokefunction","invocation_id":id,
                            "function_id":memory::hooks::TURN_COMPLETED_FN,"data":d.data,
                            "caller":d.caller,"activation":d.activation}).to_string().into())).await.unwrap();
                    }
                    frame = ws.next() => {
                        let Some(Ok(frame)) = frame else { break; };
                        let Ok(text) = frame.to_text() else { continue; };
                        let Ok(r) = serde_json::from_str::<Value>(text) else { continue; };
                        match r["type"].as_str().unwrap_or("") {
                            "registerfunction" => {
                                log.lock().unwrap().registrations.push(r.clone());
                                ws.send(Message::Text(json!({"type":"functionregistered","id":r["id"],"namespace":"default","revision":1,"local_dispatch":false}).to_string().into())).await.unwrap();
                            }
                            "observe" => {
                                ws.send(Message::Text(json!({"type":"observeresult","id":r["id"],"state":"active"}).to_string().into())).await.unwrap();
                            }
                            "unobserve" => {
                                ws.send(Message::Text(json!({"type":"observerstatechanged","id":r["id"],"state":"disposed"}).to_string().into())).await.unwrap();
                            }
                            "invocationresult" => {
                                if let Some(answer) = pending.remove(r["invocation_id"].as_str().unwrap()) { let _ = answer.send(r); }
                            }
                            "invokefunction" => {
                                let f = r["function_id"].as_str().unwrap();
                                let mode = log.lock().unwrap().mode.clone();
                                log.lock().unwrap().calls.push(r.clone());
                                let result = match f {
                                    "engine::functions::info" if mode == "owner-error" => Value::Null,
                                    "engine::functions::info" if mode == "public-source" => json!({"function_id":hook_binding::COMPLETION_SOURCE,"namespace":"default","worker_name":"source-host","private":false}),
                                    "engine::functions::info" if mode == "wrong-source" => json!({"function_id":hook_binding::SOURCE,"namespace":"default","worker_name":"source-host","private":true}),
                                    "engine::functions::info" => json!({"function_id":hook_binding::COMPLETION_SOURCE,"namespace":"default","worker_name":"source-host","private":true}),
                                    "engine::workers::info" => json!({"worker":{"id":"owner-one","namespace":"default","status":if mode == "stale-owner" {"disconnected"} else {"connected"}},"functions":[{"function_id":hook_binding::COMPLETION_SOURCE,"namespace":"default"}]}),
                                    "session::get" => {
                                        match mode.as_str() {
                                            "missing" => Value::Null,
                                            "unknown" => meta(json!("future-kind")),
                                            "malformed-kind" => meta(json!({"user":true})),
                                            "user" => meta(json!("user")),
                                            "legacy" => { let mut m = meta(json!("user")); m["meta"].as_object_mut().unwrap().remove("kind"); m },
                                            "mismatch" => { let mut m = meta(json!("automation")); m["meta"]["session_id"] = json!("other"); m },
                                            "e2e" => meta(json!("e2e")),
                                            _ => meta(json!("automation")),
                                        }
                                    }
                                    events::ON_EXTRACTION_DECISION_FN => {
                                        log.lock().unwrap().decisions.push(r["data"].clone()); r["data"].clone()
                                    }
                                    _ => Value::Null,
                                };
                                if !r["invocation_id"].is_null() {
                                    let mut reply = json!({"type":"invocationresult","invocation_id":r["invocation_id"],"function_id":f,"result":result});
                                    if (mode == "session-error" && f == "session::get") || (mode == "transport-error" && f == "engine::functions::info") {
                                        reply.as_object_mut().unwrap().remove("result");
                                        reply["error"] = json!({"code":"FIXTURE_TRANSIENT","message":"synthetic transport fault"});
                                    }
                                    ws.send(Message::Text(reply.to_string().into())).await.unwrap();
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        });
        let iii = Arc::new(iii_sdk::register_worker(
            &url,
            iii_sdk::InitOptions::default(),
        ));
        wait.await.unwrap();
        let deps = Arc::new(Deps {
            hook_binding: Arc::default(),
            iii: iii.clone(),
            emitter: Arc::new(events::Emitter::new(iii.clone())),
            store: Arc::new(RwLock::new(Arc::new(Store::open(root.into()).unwrap()))),
            config: Arc::new(RwLock::new(Arc::new(WorkerConfig {
                embeddings_enabled: false,
                extraction_enabled: true,
                ..WorkerConfig::default()
            }))),
        });
        // Seed actual FileStore rules before taking the persisted snapshot.
        let (bank, _) = deps
            .store()
            .await
            .ensure_bank("synthetic-bank", None)
            .await
            .unwrap();
        bank.set_rule("synthetic-rule", "Synthetic stable rule")
            .unwrap();
        events::register_all(&iii);
        hook_binding::register_turn_completed(&iii, &deps);
        let handle = iii.observe(
            Source::function(hook_binding::COMPLETION_SOURCE).capture([Capture::Output]),
            memory::hooks::TURN_COMPLETED_FN,
        );
        if install_binding {
            deps.hook_binding.install_completion(handle.clone());
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), handle.ready())
            .await
            .unwrap()
            .unwrap();
        iii.call(CallRequest::new("fixture::barrier", json!({})))
            .await
            .unwrap();
        Self {
            deps,
            tx,
            recorded,
            handle,
            task,
        }
    }
    async fn invoke(&self, data: Value, worker: &str, activation: Option<&str>) {
        let data = if data.get("fixture_state").is_some() {
            data
        } else {
            observed(data)
        };
        self.invoke_delivery(data, worker, activation).await;
    }
    async fn invoke_delivery(&self, data: Value, worker: &str, activation: Option<&str>) {
        let (answer, wait) = oneshot::channel();
        self.tx
            .send(Delivery {
                data,
                caller: {
                    let mode = self.recorded.lock().unwrap().mode.clone();
                    let mut caller = json!({"worker_name":if worker == "owner-one" {"iii-engine"} else {worker},
                        "function_id":if mode == "wrong-callback-source" {hook_binding::SOURCE} else {hook_binding::COMPLETION_SOURCE}});
                    if mode == "forged-engine-name" {
                        caller["worker_id"] = json!("foreign-worker");
                    }
                    caller
                },
                activation: activation
                    .map(|id| json!({"observer_id":id}))
                    .unwrap_or(Value::Null),
                answer,
            })
            .unwrap();
        let response = tokio::time::timeout(std::time::Duration::from_secs(5), wait)
            .await
            .unwrap()
            .unwrap();
        assert!(response["error"].is_null(), "{response}");
        assert_eq!(response["result"]["ok"], true);
        // Barrier includes void decision delivery in the same SDK stream.
        self.deps
            .iii
            .call(CallRequest::new("fixture::barrier", json!({})))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn authenticated_nonuser_emits_operational_only_private_typed_event() {
    let dir = tempfile::tempdir().unwrap();
    let s = Stack::new(dir.path()).await;
    let before = snapshot(dir.path());
    s.invoke(completed(), "owner-one", Some(s.handle.id()))
        .await;
    assert_eq!(
        snapshot(dir.path()),
        before,
        "actual FileStore bytes changed"
    );
    let r = s.recorded.lock().unwrap();
    assert_eq!(
        r.decisions,
        vec![
            json!({"version":1,"session_id":"synthetic-session","turn_id":"synthetic-turn","status":"skipped","reason":"session_kind_not_user","session_kind":"automation","producer_version":env!("CARGO_PKG_VERSION")})
        ]
    );
    let descriptor = r
        .registrations
        .iter()
        .find(|r| r["id"] == events::ON_EXTRACTION_DECISION_FN)
        .unwrap();
    assert_eq!(descriptor["private"], true, "{descriptor}");
    assert!(descriptor["description"].as_str().unwrap().contains("50ms"));
    assert!(descriptor["request_format"].is_object(), "{descriptor}");
    assert_eq!(descriptor["request_format"], descriptor["response_format"]);
    let allowed = [
        "engine::workers::register",
        "fixture::barrier",
        "engine::functions::info",
        "engine::workers::info",
        "session::get",
        events::ON_EXTRACTION_DECISION_FN,
    ];
    assert!(
        r.calls
            .iter()
            .all(|c| allowed.contains(&c["function_id"].as_str().unwrap())),
        "unexpected side effects: {:?}",
        r.calls
    );
    assert_eq!(r.count("session::get"), 1);
    assert_eq!(r.count("engine::functions::info"), 1);
    assert_eq!(r.count("engine::workers::info"), 1);
    assert_eq!(r.count(events::ON_EXTRACTION_DECISION_FN), 1);
    // Raw replayable envelope separates SDK metadata from the completion payload.
    println!(
        "positive_loopback_fixture={}",
        json!({"type":"invokefunction","invocation_id":"00000000-0000-4000-8000-000000000001","function_id":memory::hooks::TURN_COMPLETED_FN,"data":observed(completed()),"caller":{"worker_name":"iii-engine","function_id":hook_binding::COMPLETION_SOURCE},"activation":{"observer_id":s.handle.id()}})
    );
    println!("positive_descriptor={descriptor}");
    println!("positive_decision={}", r.decisions[0]);
    println!(
        "positive_counters={}; persisted_bytes_unchanged=true",
        serde_json::to_value(r.counters()).unwrap()
    );
}

#[tokio::test]
async fn missing_captured_output_never_looks_up_authority_or_emits_a_skip() {
    for status in ["success", "error"] {
        let dir = tempfile::tempdir().unwrap();
        let s = Stack::new(dir.path()).await;
        let before = snapshot(dir.path());
        let mut data = observed(completed());
        data["status"] = json!(status);
        data["input"] = completed();
        data["caller"] = json!({"worker_id":"owner-one","worker_name":"harness"});
        data.as_object_mut().unwrap().remove("output");
        if status == "error" {
            data["error"] = json!({"code":"FIXTURE","message":"synthetic failure"});
        }
        s.invoke_delivery(data, "owner-one", Some(s.handle.id()))
            .await;
        assert_eq!(snapshot(dir.path()), before, "{status}");
        let r = s.recorded.lock().unwrap();
        assert!(r.decisions.is_empty(), "{status}");
        assert!(
            r.counters().values().all(|count| *count == 0),
            "{status}: {:?}",
            r.counters()
        );
    }
}

#[tokio::test]
async fn payload_caller_cannot_replace_sdk_callback_authority() {
    let dir = tempfile::tempdir().unwrap();
    let s = Stack::new(dir.path()).await;
    let before = snapshot(dir.path());
    let mut data = observed(completed());
    data["caller"] =
        json!({"worker_name":"iii-engine","function_id":hook_binding::COMPLETION_SOURCE});
    s.invoke_delivery(data, "foreign", Some(s.handle.id()))
        .await;
    assert_eq!(snapshot(dir.path()), before);
    let r = s.recorded.lock().unwrap();
    assert!(r.decisions.is_empty());
    assert_eq!(r.count("engine::functions::info"), 0);
    assert_eq!(r.count("engine::workers::info"), 0);
}

#[tokio::test]
async fn untrusted_stale_unknown_and_nonterminal_deliveries_never_prove_skip() {
    let cases = [
        "direct",
        "wrong-observer",
        "foreign",
        "owner-error",
        "public-source",
        "wrong-source",
        "wrong-callback-source",
        "forged-engine-name",
        "transport-error",
        "stale-owner",
        "missing",
        "unknown",
        "malformed-kind",
        "mismatch",
        "session-error",
        "nonterminal",
        "bad-status",
        "missing-status",
        "missing-terminal",
        "blank-session",
        "subagent-claims",
        "missing-turn",
        "oversized",
        "payload-forgery",
        "inactive",
        "pending",
        "binding-error",
        "reconnect",
    ];
    for mode in cases {
        let dir = tempfile::tempdir().unwrap();
        let s = Stack::new(dir.path()).await;
        let before = snapshot(dir.path());
        s.recorded.lock().unwrap().mode = mode.into();
        let mut data = completed();
        match mode {
            "nonterminal" => data["terminal"] = json!(false),
            "bad-status" => data["status"] = json!("running"),
            "missing-status" => {
                data.as_object_mut().unwrap().remove("status");
            }
            "missing-terminal" => {
                data.as_object_mut().unwrap().remove("terminal");
            }
            "blank-session" => data["session_id"] = json!(" "),
            "subagent-claims" => {
                data["parent"] = json!({"session_id":"fake-user-parent"});
                data["kind"] = json!("user");
                data["role"] = json!("user");
            }
            "missing-turn" => {
                data.as_object_mut().unwrap().remove("turn_id");
            }
            "oversized" => data["turn_id"] = json!("x".repeat(257)),
            "payload-forgery" => {
                data["activation"] = json!({"observer_id":s.handle.id()});
                data["trusted"] = json!(true);
            }
            "inactive" | "pending" | "binding-error" | "reconnect" => {
                let state = if mode == "inactive" {
                    "disposed"
                } else if mode == "pending" {
                    "pending"
                } else if mode == "binding-error" {
                    "error"
                } else {
                    "suspended"
                };
                s.invoke(
                    json!({"fixture_state":state}),
                    "owner-one",
                    Some(s.handle.id()),
                )
                .await;
                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    while s.handle.state().to_string() != state {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
            }
            _ => {}
        }
        let observer = match mode {
            "direct" | "payload-forgery" | "subagent-claims" => None,
            "wrong-observer" => Some("wrong-observer"),
            _ => Some(s.handle.id()),
        };
        s.invoke(
            data,
            if mode == "foreign" {
                "foreign"
            } else {
                "owner-one"
            },
            observer,
        )
        .await;
        assert!(s.recorded.lock().unwrap().decisions.is_empty(), "{mode}");
        assert_eq!(
            snapshot(dir.path()),
            before,
            "{mode}: persisted side effect"
        );
        let r = s.recorded.lock().unwrap();
        println!(
            "negative_case={mode}; counters={}; persisted_bytes_unchanged=true",
            serde_json::to_value(r.counters()).unwrap()
        );
        assert!(
            !r.calls.iter().any(|c| matches!(
                c["function_id"].as_str(),
                Some(
                    "engine::queue::enqueue"
                        | "router::complete"
                        | "state::set"
                        | "memory::on-item-changed"
                        | "memory::on-bank-changed"
                )
            )),
            "{mode}: {:?}",
            r.calls
        );
    }
}

#[tokio::test]
async fn eligible_registered_calls_keep_completed_cancelled_failed_queue_backcompat() {
    for kind in ["user", "legacy"] {
        for status in ["completed", "cancelled", "failed"] {
            for binding in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let s = Stack::with_binding(dir.path(), binding).await;
                s.recorded.lock().unwrap().mode = kind.into();
                let mut data = completed();
                data["status"] = json!(status);
                s.invoke(data, "owner-one", binding.then(|| s.handle.id()))
                    .await;
                let r = s.recorded.lock().unwrap();
                assert!(r.decisions.is_empty(), "{kind}/{status}/{binding}");
                assert_eq!(
                    r.count("engine::queue::enqueue"),
                    1,
                    "{kind}/{status}/{binding}"
                );
                assert_eq!(r.count("session::get"), 1);
                // A user session's completion never needs the owner lookup.
                assert_eq!(r.count("engine::functions::info"), 0);
                assert_eq!(r.count("engine::workers::info"), 0);
                assert_eq!(r.count("router::complete"), 0);
                println!(
                    "eligible_kind={kind}; status={status}; binding={binding}; counters={}",
                    serde_json::to_value(r.counters()).unwrap()
                );
            }
        }
    }
}

#[tokio::test]
async fn subagents_return_before_session_authority_or_owner_lookup_without_proof() {
    for kind in ["user", "automation", "e2e", "legacy"] {
        for parent_field in ["parent", "parent_session_id"] {
            for binding in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let s = Stack::with_binding(dir.path(), binding).await;
                s.recorded.lock().unwrap().mode = kind.into();
                let before = snapshot(dir.path());
                let mut data = completed();
                data[parent_field] = if parent_field == "parent" {
                    json!({"session_id":"synthetic-parent"})
                } else {
                    json!("synthetic-parent")
                };
                s.invoke(data, "owner-one", binding.then(|| s.handle.id()))
                    .await;
                assert_eq!(snapshot(dir.path()), before);
                let r = s.recorded.lock().unwrap();
                assert!(r.decisions.is_empty(), "{kind}/{parent_field}/{binding}");
                assert!(
                    r.counters().values().all(|count| *count == 0),
                    "{kind}/{parent_field}/{binding}: {:?}",
                    r.counters()
                );
                assert!(
                    r.calls.iter().all(|c| matches!(
                        c["function_id"].as_str(),
                        Some("engine::workers::register" | "fixture::barrier")
                    )),
                    "unexpected RPC: {:?}",
                    r.calls
                );
                println!("subagent_kind={kind}; field={parent_field}; binding={binding}; counters={}; persisted_bytes_unchanged=true", serde_json::to_value(r.counters()).unwrap());
            }
        }
    }
}

#[tokio::test]
async fn e2e_kind_is_authoritative_and_duplicates_remain_possible() {
    let dir = tempfile::tempdir().unwrap();
    let s = Stack::new(dir.path()).await;
    s.recorded.lock().unwrap().mode = "e2e".into();
    for _ in 0..2 {
        s.invoke(completed(), "owner-one", Some(s.handle.id()))
            .await;
    }
    let r = s.recorded.lock().unwrap();
    assert_eq!(r.decisions.len(), 2);
    assert_eq!(r.decisions[0]["session_kind"], "e2e");
    assert_eq!(r.decisions[0], r.decisions[1]);
}
