//! SDK loopback with MOCK engine stamping, not a universal engine ACL proof.
//! No model, live service or real Memory data.
use futures_util::{SinkExt, StreamExt};
use iii_sdk::{CallRequest, Source};
use memory::{config::WorkerConfig, deps::Deps, events::Emitter, hook_binding, store::Store};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio_tungstenite::tungstenite::Message;

struct Request {
    input: Value,
    caller: Value,
    activation: Value,
    answer: oneshot::Sender<Value>,
}
#[derive(Default)]
struct Fixture {
    path: Vec<Value>,
    lookup: String,
    queued: Vec<Value>,
    cursor: Value,
    prompts: Vec<String>,
    calls: Vec<String>,
}
struct Stack {
    deps: Arc<Deps>,
    tx: mpsc::UnboundedSender<Request>,
    owner: Arc<Mutex<String>>,
    fixture: Arc<Mutex<Fixture>>,
    server: tokio::task::JoinHandle<()>,
    handle: iii_sdk::ObserverHandle,
}
impl Drop for Stack {
    fn drop(&mut self) {
        self.deps.iii.shutdown();
        self.server.abort();
    }
}
fn source() -> Value {
    json!({"entry_id":"opaque-entry","message":{"role":"user","content":[{"type":"text","text":"abc"}]}})
}
fn input(turn: &str) -> Value {
    json!({"session_id":"synthetic-session","turn_id":turn,"recall_context":{
    "version":1,"session_id":"synthetic-session","turn_id":turn,
    "source":{"kind":"direct","entry_id":"opaque-entry"},
    "extraction_history":{"version":1,"retention":{"delivery_entries":128,"turns_per_entry":8},
    "missing":"unknown","rows":[{"source_kind":"direct","attestation":{
        "version":1,"session_id":"synthetic-session","turn_id":format!("old-{turn}"),
        "entry_id":"opaque-entry","text_sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        "policy":"on","bank":"old-bank","terminal":{"status":"failed","exchange_finished":true}
    }}]}}})
}
fn key() -> String {
    json!(["authenticated-history-v1", "synthetic-session"]).to_string()
}
impl Stack {
    async fn new(root: &std::path::Path) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel::<Request>();
        let owner = Arc::new(Mutex::new("owner-one".to_string()));
        let host = owner.clone();
        let fixture = Arc::new(Mutex::new(Fixture {
            path: vec![source()],
            ..Fixture::default()
        }));
        let replies = fixture.clone();
        let (ready, wait) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            socket.send(Message::Text(json!({"type":"workerregistered","worker_id":"memory-fixture","features":["observe.v2"]}).to_string().into())).await.unwrap();
            let _ = ready.send(());
            let mut seq = 0u64;
            let mut pending = std::collections::HashMap::new();
            loop {
                tokio::select! {
                    command = rx.recv() => {
                        let Some(command) = command else { break; };
                        if let Some(state) = command.input["fixture_state"].as_str() {
                            socket.send(Message::Text(json!({"type":"observerstatechanged",
                                "id":command.activation["observer_id"],"state":state}).to_string().into())).await.unwrap();
                            let _ = command.answer.send(json!({"result":{"decision":"continue"}}));
                            continue;
                        }
                        seq += 1;
                        let id = format!("00000000-0000-4000-8000-{seq:012}");
                        pending.insert(id.clone(),command.answer);
                        socket.send(Message::Text(json!({"type":"invokefunction","invocation_id":id,
                            "function_id":memory::hooks::PRE_GENERATE_FN,"data":command.input,
                            "caller":command.caller,"activation":command.activation}).to_string().into())).await.unwrap();
                    }
                    frame = socket.next() => {
                        let Some(Ok(frame)) = frame else { break; };
                        let Ok(text) = frame.to_text() else { continue; };
                        let Ok(request) = serde_json::from_str::<Value>(text) else { continue; };
                        match request["type"].as_str().unwrap_or("") {
                            "registerfunction" => { socket.send(Message::Text(json!({"type":"functionregistered","id":request["id"],"namespace":"default","revision":1,"local_dispatch":false}).to_string().into())).await.unwrap(); }
                            "observe" => { socket.send(Message::Text(json!({"type":"observeresult","id":request["id"],"state":"active"}).to_string().into())).await.unwrap(); }
                            "unobserve" => { socket.send(Message::Text(json!({"type":"observerstatechanged","id":request["id"],"state":"disposed"}).to_string().into())).await.unwrap(); }
                            "invocationresult" => {
                                if let Some(answer) = pending.remove(request["invocation_id"].as_str().unwrap()) { let _ = answer.send(request); }
                            }
                            "invokefunction" => {
                                let function = request["function_id"].as_str().unwrap();
                                replies.lock().unwrap().calls.push(function.to_owned());
                                let result = match function {
                                    "engine::functions::info" if replies.lock().unwrap().lookup == "function" => Value::Null,
                                    "engine::workers::info" if replies.lock().unwrap().lookup == "worker" => Value::Null,
                                    "engine::functions::info" => json!({"function_id":hook_binding::SOURCE,"namespace":"default","worker_name":"actual-source-host"}),
                                    "engine::workers::info" => json!({"worker":{"id":host.lock().unwrap().clone(),"status":"connected","namespace":"default"},"functions":[{"function_id":hook_binding::SOURCE,"namespace":"default"}]}),
                                    "session::messages" => {
                                        let fixture = replies.lock().unwrap();
                                        let start = request["data"]["after_entry_id"].as_str()
                                            .and_then(|id| fixture.path.iter().position(|p| p["entry_id"] == id)).map(|n|n+1)
                                            .unwrap_or_else(|| request["data"]["cursor"].as_str().and_then(|s| s.parse::<usize>().ok()).unwrap_or(0));
                                        let end = (start+100).min(fixture.path.len());
                                        json!({"messages":fixture.path[start..end],"next_cursor":(end < fixture.path.len()).then(||end.to_string())})
                                    },
                                    "session::messages-tail" => json!({"messages":replies.lock().unwrap().path}),
                                    "engine::queue::enqueue" => {
                                        replies.lock().unwrap().queued.push(request["data"]["data"].clone());
                                        json!({"ok":true})
                                    },
                                    "state::get" => replies.lock().unwrap().cursor.clone(),
                                    "state::set" => { replies.lock().unwrap().cursor = request["data"]["value"].clone(); json!({"ok":true}) },
                                    "router::complete" => {
                                        replies.lock().unwrap().prompts.push(request["data"].to_string());
                                        json!({"message":{"role":"assistant","stop_reason":"stop","content":[{"type":"text","text":"[{\"text\":\"The user prefers copper instruments.\",\"entities\":[\"user\"],\"tags\":[],\"confidence\":\"extracted\",\"kind\":\"memory\",\"scope\":\"project\"}]"}]}})
                                    },
                                    "session::get" => json!({"meta":{"session_id":"synthetic-session","kind":"user","metadata":{}}}),
                                    _ => Value::Null,
                                };
                                if !request["invocation_id"].is_null() {
                                    socket.send(Message::Text(json!({"type":"invocationresult","invocation_id":request["invocation_id"],"function_id":function,"result":result}).to_string().into())).await.unwrap();
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
            emitter: Arc::new(Emitter::new(iii.clone())),
            iii: iii.clone(),
            store: Arc::new(RwLock::new(Arc::new(Store::open(root.into()).unwrap()))),
            config: Arc::new(RwLock::new(Arc::new(WorkerConfig {
                inject_rules: false,
                inject_memories: false,
                embeddings_enabled: false,
                extraction_enabled: false,
                ..WorkerConfig::default()
            }))),
        });
        hook_binding::register_pre_generate(&iii, &deps);
        let handle = iii.observe(
            Source::function(hook_binding::SOURCE),
            memory::hooks::PRE_GENERATE_FN,
        );
        deps.hook_binding.install(handle.clone());
        tokio::time::timeout(std::time::Duration::from_secs(5), handle.ready())
            .await
            .unwrap()
            .unwrap();
        // Ordered barrier guarantees registration frames were received.
        iii.call(CallRequest::new("fixture::barrier", json!({})))
            .await
            .unwrap();
        Self {
            deps,
            tx,
            owner,
            fixture,
            server,
            handle,
        }
    }
    async fn invoke(&self, input: Value, worker: &str, observer: Option<&str>) -> Value {
        let (answer, wait) = oneshot::channel();
        self.tx.send(Request { input,
            caller: json!({"worker_id":worker,"worker_name":"harness","parent":{"worker_id":"forged-parent"}}),
            activation: observer.map(|id|json!({"observer_id":id})).unwrap_or(Value::Null),answer }).unwrap();
        let response = tokio::time::timeout(std::time::Duration::from_secs(5), wait)
            .await
            .unwrap()
            .unwrap();
        // The hook answers first; its history import runs in the background.
        self.deps.hook_binding.import_settled().await;
        assert!(response["error"].is_null(), "{response}");
        assert_eq!(response["result"]["decision"], "continue");
        response
    }
}

#[tokio::test]
async fn registered_receiver_persists_only_actual_active_owner_activation_and_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let stack = Stack::new(dir.path()).await;
    // Forwarded agent shares Harness's client, but has no correct activation.
    stack.invoke(input("direct"), "owner-one", None).await;
    stack
        .invoke(input("wrong"), "owner-one", Some("wrong-observer"))
        .await;
    stack
        .invoke(input("foreign"), "foreign", Some(stack.handle.id()))
        .await;
    assert!(stack
        .deps
        .store()
        .await
        .extraction_plan(&key())
        .unwrap()
        .is_none());
    stack
        .invoke(input("valid"), "owner-one", Some(stack.handle.id()))
        .await;
    let store = stack.deps.store().await;
    let rows = store.extraction_plan(&key()).unwrap().unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["attestation"]["bank"], "old-bank");
    assert!(!rows.to_string().contains("\"text\""));
    drop(store);
    // Owner rotates under the same source name: old connection is rejected.
    *stack.owner.lock().unwrap() = "owner-two".into();
    stack
        .invoke(input("stale"), "owner-one", Some(stack.handle.id()))
        .await;
    assert_eq!(
        stack.deps.store().await.extraction_plan(&key()).unwrap(),
        Some(rows.clone())
    );
    stack
        .invoke(input("rotated"), "owner-two", Some(stack.handle.id()))
        .await;
    assert_eq!(
        stack
            .deps
            .store()
            .await
            .extraction_plan(&key())
            .unwrap()
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let replacement = stack.deps.iii.observe(
        Source::function(hook_binding::SOURCE),
        memory::hooks::PRE_GENERATE_FN,
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), replacement.ready())
        .await
        .unwrap()
        .unwrap();
    stack.deps.hook_binding.install(replacement.clone());
    stack
        .invoke(input("stale-id"), "owner-two", Some(stack.handle.id()))
        .await;
    assert_eq!(
        stack
            .deps
            .store()
            .await
            .extraction_plan(&key())
            .unwrap()
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    stack
        .invoke(input("replacement"), "owner-two", Some(replacement.id()))
        .await;
    assert_eq!(
        stack
            .deps
            .store()
            .await
            .extraction_plan(&key())
            .unwrap()
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        3
    );
    replacement.dispose().await.unwrap();
    stack.handle.dispose().await.unwrap();
    let before = stack.deps.store().await.extraction_plan(&key()).unwrap();
    stack
        .invoke(input("inactive"), "owner-two", Some(stack.handle.id()))
        .await;
    assert_eq!(
        stack.deps.store().await.extraction_plan(&key()).unwrap(),
        before
    );
    drop(stack);
    let reopened = Store::open(dir.path().into()).unwrap();
    assert_eq!(reopened.extraction_plan(&key()).unwrap(), before);
}

#[tokio::test]
async fn registered_receiver_rejects_digest_fork_bounds_and_preserves_missing_backlog() {
    let dir = tempfile::tempdir().unwrap();
    let stack = Stack::new(dir.path()).await;
    let mut forged = input("digest");
    forged["recall_context"]["extraction_history"]["rows"][0]["attestation"]["text_sha256"] =
        json!("0".repeat(64));
    stack
        .invoke(forged, "owner-one", Some(stack.handle.id()))
        .await;
    assert!(stack
        .deps
        .store()
        .await
        .extraction_plan(&key())
        .unwrap()
        .is_none());
    let mut wrong_session = input("session");
    wrong_session["recall_context"]["extraction_history"]["rows"][0]["attestation"]["session_id"] =
        json!("fork");
    stack
        .invoke(wrong_session, "owner-one", Some(stack.handle.id()))
        .await;
    assert!(stack
        .deps
        .store()
        .await
        .extraction_plan(&key())
        .unwrap()
        .is_none());
    stack
        .invoke(input("valid"), "owner-one", Some(stack.handle.id()))
        .await;
    let before = stack.deps.store().await.extraction_plan(&key()).unwrap();
    let mut missing = input("missing");
    missing["recall_context"]["extraction_history"]["rows"] = json!([]);
    stack
        .invoke(missing, "owner-one", Some(stack.handle.id()))
        .await;
    assert_eq!(
        stack.deps.store().await.extraction_plan(&key()).unwrap(),
        before
    );
    for (turn, policy, bank, kind) in [
        ("off", "off", "old-bank", "direct"),
        ("unknown", "unknown", "old-bank", "direct"),
        ("notification", "on", "new-bank", "notification"),
    ] {
        let mut delivery = input(turn);
        let row = &mut delivery["recall_context"]["extraction_history"]["rows"][0];
        row["source_kind"] = json!(kind);
        row["attestation"]["policy"] = json!(policy);
        row["attestation"]["bank"] = json!(bank);
        stack
            .invoke(delivery, "owner-one", Some(stack.handle.id()))
            .await;
    }
    let quarantined = stack
        .deps
        .store()
        .await
        .extraction_plan(&key())
        .unwrap()
        .unwrap();
    assert_eq!(quarantined.as_array().unwrap().len(), 4);
    assert_eq!(quarantined[1]["attestation"]["policy"], "off");
    assert_eq!(quarantined[2]["attestation"]["policy"], "unknown");
    assert_eq!(quarantined[3]["source_kind"], "notification");
    assert_eq!(quarantined[3]["attestation"]["bank"], "new-bank");
    // None of these rows is a queue authorization or a source-origin upgrade.
    assert!(stack
        .deps
        .store()
        .await
        .extraction_ranges("synthetic-session")
        .unwrap()
        .is_empty());
    let before = Some(quarantined);
    let mut invalid = input("bounds");
    invalid["recall_context"]["extraction_history"]["retention"]["delivery_entries"] = json!(129);
    stack
        .invoke(invalid, "owner-one", Some(stack.handle.id()))
        .await;
    assert_eq!(
        stack.deps.store().await.extraction_plan(&key()).unwrap(),
        before
    );
}

#[tokio::test]
async fn registered_receiver_fails_closed_for_states_and_lookups() {
    let dir = tempfile::tempdir().unwrap();
    let stack = Stack::new(dir.path()).await;
    for state in ["pending", "suspended", "error"] {
        stack
            .invoke(
                json!({"fixture_state":state}),
                "owner-one",
                Some(stack.handle.id()),
            )
            .await;
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while stack.handle.state().to_string() != state {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let before = stack.deps.store().await.extraction_plan(&key()).unwrap();
        stack
            .invoke(input(state), "owner-one", Some(stack.handle.id()))
            .await;
        assert_eq!(
            stack.deps.store().await.extraction_plan(&key()).unwrap(),
            before
        );
    }
    stack
        .invoke(
            json!({"fixture_state":"active"}),
            "owner-one",
            Some(stack.handle.id()),
        )
        .await;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while stack.handle.state() != iii_sdk::ObserverState::Active {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    stack.handle.ready().await.unwrap();
    // Authenticated once per delivery: an owner swap after that check does
    // not reject the delivery the authenticated owner sent.
    for failure in ["function", "worker"] {
        stack.fixture.lock().unwrap().lookup = failure.into();
        let before = stack.deps.store().await.extraction_plan(&key()).unwrap();
        stack
            .invoke(input(failure), "owner-one", Some(stack.handle.id()))
            .await;
        assert_eq!(
            stack.deps.store().await.extraction_plan(&key()).unwrap(),
            before
        );
    }
    assert!(stack.fixture.lock().unwrap().queued.is_empty());
}

#[tokio::test]
async fn a_delivery_with_nothing_new_to_recover_costs_no_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let stack = Stack::new(dir.path()).await;
    let looked_up = || {
        let calls = std::mem::take(&mut stack.fixture.lock().unwrap().calls);
        calls
            .iter()
            .any(|c| c == "engine::functions::info" || c == "session::messages")
    };
    let mut completed = input("completed");
    completed["recall_context"]["extraction_history"]["rows"][0]["attestation"]["terminal"]
        ["status"] = json!("completed");
    stack
        .invoke(completed, "owner-one", Some(stack.handle.id()))
        .await;
    assert!(!looked_up());
    assert!(stack
        .deps
        .store()
        .await
        .extraction_plan(&key())
        .unwrap()
        .is_none());
    stack
        .invoke(input("failed"), "owner-one", Some(stack.handle.id()))
        .await;
    assert!(looked_up());
    let stored = stack.deps.store().await.extraction_plan(&key()).unwrap();
    // The same failed request delivered again on the next step is already held.
    stack
        .invoke(input("failed"), "owner-one", Some(stack.handle.id()))
        .await;
    assert!(!looked_up());
    assert_eq!(
        stack.deps.store().await.extraction_plan(&key()).unwrap(),
        stored
    );
}

#[tokio::test]
async fn history_rows_the_cursor_passed_are_dropped_on_import() {
    let dir = tempfile::tempdir().unwrap();
    let stack = Stack::new(dir.path()).await;
    stack
        .invoke(input("before"), "owner-one", Some(stack.handle.id()))
        .await;
    let rows = stack.deps.store().await.extraction_plan(&key()).unwrap();
    assert_eq!(rows.unwrap().as_array().unwrap().len(), 1);
    // The cursor passes the attested entry: the stored row goes, and the
    // same row delivered again is not kept.
    stack.fixture.lock().unwrap().cursor = json!({"last_entry_id":"opaque-entry"});
    stack
        .invoke(input("after"), "owner-one", Some(stack.handle.id()))
        .await;
    assert_eq!(
        stack.deps.store().await.extraction_plan(&key()).unwrap(),
        None
    );
}

#[tokio::test]
async fn authenticated_failed_request_recovers_original_bank_after_restart_and_excludes_off() {
    for policy in ["on", "off", "unknown"] {
        let dir = tempfile::tempdir().unwrap();
        let stack = Stack::new(dir.path()).await;
        let text = "The user prefers copper instruments.";
        let old = json!({"entry_id":"opaque-entry","message":{"role":"user","timestamp":1700000000000u64,"content":[{"type":"text","text":text}]}});
        let next = json!({"entry_id":"next-request","message":{"role":"user","timestamp":1700000001000u64,"content":[{"type":"text","text":"The next request belongs only to the next bank."}]}});
        let end = json!({"entry_id":"next-end","message":{"role":"assistant","timestamp":1700000002000u64,"content":[{"type":"text","text":"Next answer."}]},"origin":{"turn_id":"next-turn","memory_extraction":true,"memory_bank":"new-bank"}});
        stack.fixture.lock().unwrap().path = vec![old, next, end];
        let mut cfg = (*stack.deps.config().await).clone();
        cfg.extraction_enabled = true;
        cfg.extraction_model = "fixture-only".into();
        *stack.deps.config.write().await = Arc::new(cfg);
        let mut delivery = input("now");
        let a = &mut delivery["recall_context"]["extraction_history"]["rows"][0]["attestation"];
        a["policy"] = json!(policy);
        a["text_sha256"] =
            json!("75a1e6098f1a3f42d47fb37d2f317b2b836b7798edbd06b88d37c0114b6ae438");
        stack
            .invoke(delivery, "owner-one", Some(stack.handle.id()))
            .await;
        let queued = stack.fixture.lock().unwrap().queued.clone();
        assert_eq!(queued.len(), usize::from(policy == "on"));
        assert!(queued
            .iter()
            .all(|v| v.get("history").is_none() && v.get("extraction_history").is_none()));
        // A fresh Store reads the exact authenticated proof, never the public job.
        *stack.deps.store.write().await = Arc::new(Store::open(dir.path().into()).unwrap());
        let next_job = memory::hooks::ExtractJobInput {
            session_id: "synthetic-session".into(),
            turn_id: Some("next-turn".into()),
            bank: Some("new-bank".into()),
            until: Some("next-end".into()),
            ..Default::default()
        };
        // An unknown policy is OFF: the request closes unextracted.
        memory::extract::try_run(&stack.deps, &next_job)
            .await
            .unwrap();
        let prompts = stack.fixture.lock().unwrap().prompts.clone();
        if policy == "on" {
            // The later job runs the failed request first, under the bank of
            // the turn that failed, never under its own.
            assert_eq!(prompts.len(), 2);
            assert!(prompts[0].contains(text));
            assert!(!prompts[1].contains(text));
            let bank = stack.deps.store().await.bank("old-bank").await.unwrap();
            let (items, count) = bank.list(10, 0, false, None).await;
            assert_eq!(count, 1);
            assert_eq!(items[0].text, text);
            // The failed turn's own job, delivered late, finds nothing left.
            let old_job = serde_json::from_value(queued[0].clone()).unwrap();
            memory::extract::try_run(&stack.deps, &old_job)
                .await
                .unwrap();
            assert_eq!(stack.fixture.lock().unwrap().prompts.len(), 2);
        } else {
            assert_eq!(prompts.len(), 1);
            assert!(!prompts[0].contains(text));
            assert!(!dir.path().join("old-bank/memories.jsonl").exists());
        }
        assert_eq!(
            stack.fixture.lock().unwrap().cursor,
            json!({"last_entry_id":"next-end"})
        );
    }
}

#[tokio::test]
async fn registered_receiver_large_source_keeps_full_digest_and_later_owned_progress() {
    for (content, digest) in [
        (
            json!([{"type":"text","text":"a".repeat(1000000)}]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0",
        ),
        // Oracle: Perl emits exact UTF-8 bytes and escapes into openssl dgst.
        (
            json!([{"type":"text","text":"\u{e9}".repeat(60000)},
            {"type":"file","path":"ignored"},
            {"type":"text","text":format!("{}{}", "\u{0001}\"\\", "\n\u{0001}\"\\".repeat(9999))}]),
            "01364dc84e9c5385a5275e9b3a153a8626f4f09dff14310160e246ffd079a9ca",
        ),
    ] {
        for policy in ["on", "off", "unknown"] {
            let dir = tempfile::tempdir().unwrap();
            let stack = Stack::new(dir.path()).await;
            let mut cfg = (*stack.deps.config().await).clone();
            cfg.extraction_model = "fixture-only".into();
            *stack.deps.config.write().await = Arc::new(cfg);
            let mut item = source();
            item["message"]["content"] = content.clone();
            item["message"]["timestamp"] = json!(1700000000000u64);
            stack.fixture.lock().unwrap().path = vec![item.clone()];
            let mut delivery = input("large");
            let a = &mut delivery["recall_context"]["extraction_history"]["rows"][0]["attestation"];
            a["text_sha256"] = json!(digest);
            a["policy"] = json!(policy);
            stack
                .invoke(delivery.clone(), "owner-one", Some(stack.handle.id()))
                .await;
            let proof = stack.deps.store().await.extraction_plan(&key()).unwrap();
            assert!(
                proof.is_some(),
                "native receiver authenticates full bounded source"
            );
            for bad in ["digest", "missing-digest", "path", "tail"] {
                let mut invalid = delivery.clone();
                let mut changed = item.clone();
                match bad {
                    "digest" => {
                        invalid["recall_context"]["extraction_history"]["rows"][0]["attestation"]
                            ["text_sha256"] = json!("0".repeat(64))
                    }
                    "missing-digest" => {
                        invalid["recall_context"]["extraction_history"]["rows"][0]["attestation"]
                            .as_object_mut()
                            .unwrap()
                            .remove("text_sha256");
                    }
                    "path" => changed["entry_id"] = json!("other-path"),
                    "tail" => {
                        changed["message"]["content"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!({"type":"text","text":"changed-tail"}));
                    }
                    _ => unreachable!(),
                }
                stack.fixture.lock().unwrap().path = vec![changed];
                stack
                    .invoke(invalid, "owner-one", Some(stack.handle.id()))
                    .await;
                assert_eq!(
                    stack.deps.store().await.extraction_plan(&key()).unwrap(),
                    proof
                );
                assert!(stack.fixture.lock().unwrap().cursor.is_null());
            }
            let mut next = source();
            next["entry_id"] = json!("next-request");
            next["message"]["timestamp"] = json!(1700000001000u64);
            next["message"]["content"] =
                json!([{"type":"text","text":"I prefer copper instruments for calibration."}]);
            let end = json!({"entry_id":"next-end","origin":{"turn_id":"next-turn","memory_extraction":true,"memory_bank":"new-bank"},
                "message":{"role":"assistant","timestamp":1700000002000u64,"content":[{"type":"text","text":"Noted."}]}});
            stack.fixture.lock().unwrap().path = vec![item.clone(), next, end];
            let later = memory::hooks::ExtractJobInput {
                session_id: "synthetic-session".into(),
                turn_id: Some("next-turn".into()),
                bank: Some("new-bank".into()),
                until: Some("next-end".into()),
                ..Default::default()
            };
            memory::extract::try_run(&stack.deps, &later).await.unwrap();
            let ranges = stack
                .deps
                .store()
                .await
                .extraction_ranges("synthetic-session")
                .unwrap();
            // The large request is read from its head and tail under its
            // own turn's bank; the cursor moves past both turns.
            let on = policy == "on";
            let prompts = stack.fixture.lock().unwrap().prompts.clone();
            assert_eq!(prompts.len(), usize::from(on) + 1);
            assert!(prompts.iter().all(|prompt| prompt.len() < 64 * 1024));
            assert_eq!(dir.path().join("old-bank/memories.jsonl").exists(), on);
            assert_eq!(
                stack.fixture.lock().unwrap().cursor,
                json!({"last_entry_id":"next-end"})
            );
            assert!(ranges.is_empty());
            if on {
                *stack.deps.store.write().await = Arc::new(Store::open(dir.path().into()).unwrap());
                memory::extract::try_run(&stack.deps, &later).await.unwrap();
                let old = memory::hooks::ExtractJobInput {
                    session_id: "synthetic-session".into(),
                    turn_id: Some("old-large".into()),
                    bank: Some("old-bank".into()),
                    until: Some("opaque-entry".into()),
                    ..Default::default()
                };
                memory::extract::try_run(&stack.deps, &old).await.unwrap();
                assert_eq!(
                    stack.fixture.lock().unwrap().prompts.len(),
                    2,
                    "a redelivered job asks the model nothing"
                );
            }
        }
    }
}

#[tokio::test]
async fn registered_receiver_validates_full_128_by_8_digest_blocks_and_atomic_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let stack = Stack::new(dir.path()).await;
    let mut path = Vec::new();
    let mut rows = Vec::new();
    for entry in 0..128 {
        let mut item = source();
        item["entry_id"] = json!(format!("entry-{entry}"));
        // Empty text contributes a separator; nontext blocks contribute nothing.
        item["message"]["content"] = json!([
            {"type":"text","text":""},{"type":"file","path":"synthetic-only"},
            {"type":"text","text":"abc"}]);
        assert_eq!(
            item["message"]["content"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|block| block["type"] == "text")
                .map(|block| block["text"].as_str().unwrap())
                .collect::<Vec<_>>()
                .join("\n"),
            "\nabc"
        );
        path.push(item);
        for turn in 0..8 {
            let mut row =
                input("template")["recall_context"]["extraction_history"]["rows"][0].clone();
            // Independently verified with printf '\nabc' | openssl dgst -sha256.
            row["attestation"]["text_sha256"] =
                json!("f39d39bfc99287b848780d4110eb663f0e7c344ae41f46a390c4135b89dc2756");
            row["attestation"]["entry_id"] = json!(format!("entry-{entry}"));
            row["attestation"]["turn_id"] = json!(format!("turn-{entry}-{turn}"));
            rows.push(row);
        }
    }
    stack.fixture.lock().unwrap().path = path.clone();
    let mut delivery = input("full");
    delivery["recall_context"]["source"]["entry_id"] = json!("entry-127");
    delivery["recall_context"]["extraction_history"]["rows"] = json!(rows);
    stack
        .invoke(delivery.clone(), "owner-one", Some(stack.handle.id()))
        .await;
    let before = stack.deps.store().await.extraction_plan(&key()).unwrap();
    assert_eq!(before.as_ref().unwrap().as_array().unwrap().len(), 1024);
    for bad in ["turn", "terminal", "source", "entry-count", "turn-count"] {
        let mut invalid = delivery.clone();
        match bad {
            "turn" => {
                invalid["recall_context"]["extraction_history"]["rows"][1]["attestation"]
                    ["turn_id"] = json!("turn-0-0")
            }
            "terminal" => {
                invalid["recall_context"]["extraction_history"]["rows"][0]["attestation"]
                    ["terminal"]["status"] = json!("running")
            }
            "source" => {
                invalid["recall_context"]["extraction_history"]["rows"][0]["source_kind"] =
                    json!("public-payload")
            }
            "entry-count" => invalid["recall_context"]["extraction_history"]["rows"]
                .as_array_mut()
                .unwrap()
                .push(rows[0].clone()),
            "turn-count" => {
                invalid["recall_context"]["extraction_history"]["rows"][8]["attestation"]
                    ["entry_id"] = json!("entry-0")
            }
            _ => unreachable!(),
        }
        stack
            .invoke(invalid, "owner-one", Some(stack.handle.id()))
            .await;
        assert_eq!(
            stack.deps.store().await.extraction_plan(&key()).unwrap(),
            before,
            "{bad}"
        );
    }
    // Full-text digest distinguishes identical heads with different tails.
    let tail_dir = tempfile::tempdir().unwrap();
    let tail = Stack::new(tail_dir.path()).await;
    let mut item = source();
    item["message"]["content"] = json!([{"type":"text","text":format!("{}TAIL","a".repeat(4000))}]);
    tail.fixture.lock().unwrap().path = vec![item.clone()];
    let mut full = input("tail");
    full["recall_context"]["extraction_history"]["rows"][0]["attestation"]["text_sha256"] =
        json!("c34ed5fb006c23f4fbfea8accb208ee8fd8dd75e5a78642b3a34ddfcfac75d8e");
    tail.invoke(full.clone(), "owner-one", Some(tail.handle.id()))
        .await;
    let before = tail.deps.store().await.extraction_plan(&key()).unwrap();
    assert!(before.is_some());
    item["message"]["content"][0]["text"] = json!(format!("{}FORK", "a".repeat(4000)));
    tail.fixture.lock().unwrap().path = vec![item];
    tail.invoke(full, "owner-one", Some(tail.handle.id())).await;
    assert_eq!(
        tail.deps.store().await.extraction_plan(&key()).unwrap(),
        before
    );
}

#[tokio::test]
async fn registered_receiver_byte_rejections_preserve_store_cursor_and_allow_retry() {
    let dir = tempfile::tempdir().unwrap();
    let stack = Stack::new(dir.path()).await;
    stack
        .invoke(input("baseline"), "owner-one", Some(stack.handle.id()))
        .await;
    let before = stack.deps.store().await.extraction_plan(&key()).unwrap();
    let pending = json!({"version":1,"source_digest":"0123456789abcdef0123456789abcdef","entries":[["saved","user","retained source",1]],"bank":"old-bank","derived":false,"agent":null,"turn_id":"saved-turn","attempts":0});
    stack
        .deps
        .store()
        .await
        .set_pending_extraction("synthetic-session", pending.clone())
        .unwrap();
    let retained = memory::store::PendingExtraction::parse(pending.clone()).unwrap();
    stack
        .deps
        .store()
        .await
        .retain_extraction_range("synthetic-session", retained, false)
        .unwrap();
    let receipts = stack
        .deps
        .store()
        .await
        .extraction_ranges("synthetic-session")
        .unwrap();
    stack.fixture.lock().unwrap().cursor = json!({"last_entry_id":"saved-cursor"});
    for bad in [
        "row",
        "unknown",
        "metadata",
        "aggregate",
        "id",
        "overflow",
        "duplicate",
    ] {
        let mut delivery = input(bad);
        match bad {
            "row" => {
                delivery["recall_context"]["extraction_history"]["rows"][0]["attestation"]["bank"] =
                    json!("x".repeat(17000))
            }
            "unknown" => {
                delivery["recall_context"]["extraction_history"]["unknown"] =
                    json!("x".repeat(710000))
            }
            "metadata" => delivery["recall_context"]["metadata"] = json!("x".repeat(710000)),
            "id" => {
                delivery["recall_context"]["extraction_history"]["rows"][0]["attestation"]
                    ["turn_id"] = json!("x".repeat(257))
            }
            "overflow" => {
                delivery["recall_context"]["extraction_history"]["retention"]["delivery_entries"] =
                    json!(u64::MAX)
            }
            "duplicate" => {
                let row = delivery["recall_context"]["extraction_history"]["rows"][0].clone();
                delivery["recall_context"]["extraction_history"]["rows"] = json!([row, row]);
            }
            "aggregate" => {
                let template = delivery["recall_context"]["extraction_history"]["rows"][0].clone();
                let rows: Vec<_> = (0..1024)
                    .map(|n| {
                        let mut row = template.clone();
                        row["attestation"]["turn_id"] = json!(format!("{n:04}{}", "t".repeat(252)));
                        for field in ["bank", "layer", "agent"] {
                            row["attestation"][field] = json!("b".repeat(256));
                        }
                        row
                    })
                    .collect();
                delivery["recall_context"]["extraction_history"]["rows"] = json!(rows);
            }
            _ => unreachable!(),
        }
        stack
            .invoke(delivery, "owner-one", Some(stack.handle.id()))
            .await;
        assert_eq!(
            stack.deps.store().await.extraction_plan(&key()).unwrap(),
            before,
            "{bad}"
        );
        assert_eq!(
            stack
                .deps
                .store()
                .await
                .pending_extraction("synthetic-session")
                .unwrap(),
            Some(pending.clone())
        );
        assert_eq!(
            stack
                .deps
                .store()
                .await
                .extraction_ranges("synthetic-session")
                .unwrap(),
            receipts
        );
        assert_eq!(
            stack.fixture.lock().unwrap().cursor,
            json!({"last_entry_id":"saved-cursor"})
        );
        assert!(stack.fixture.lock().unwrap().queued.is_empty());
        assert!(stack.fixture.lock().unwrap().prompts.is_empty());
    }
    // Failed delivery cannot poison an otherwise valid subsequent turn.
    stack
        .invoke(input("retry"), "owner-one", Some(stack.handle.id()))
        .await;
    let reopened = Store::open(dir.path().into()).unwrap();
    assert_eq!(
        reopened
            .extraction_plan(&key())
            .unwrap()
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        reopened.pending_extraction("synthetic-session").unwrap(),
        Some(pending)
    );
}

#[tokio::test]
async fn registered_receiver_caps_full_join_and_path_before_copying_without_prefix_certification() {
    let dir = tempfile::tempdir().unwrap();
    let stack = Stack::new(dir.path()).await;
    // Literal external SHA-256 oracle for exactly 96KiB of 'a'.
    let mut boundary = source();
    boundary["message"]["content"] = json!([{"type":"text","text":"a".repeat(96 * 1024)}]);
    stack.fixture.lock().unwrap().path = vec![boundary.clone()];
    let mut delivery = input("boundary");
    delivery["recall_context"]["extraction_history"]["rows"][0]["attestation"]["text_sha256"] =
        json!("6e03ec5e819aa1579fd8d3e35ea4a88cddbedc2707f1bae3fd3c08710a515831");
    stack
        .invoke(delivery.clone(), "owner-one", Some(stack.handle.id()))
        .await;
    let before = stack.deps.store().await.extraction_plan(&key()).unwrap();
    assert!(before.is_some());
    for bad in ["text", "blocks", "metadata", "path", "fork"] {
        let mut item = boundary.clone();
        let mut path = Vec::new();
        match bad {
            "text" => item["message"]["content"][0]["text"] = json!("a".repeat(1024 * 1024 + 1)),
            // Individual blocks fit; separators plus aggregate text do not.
            "blocks" => {
                item["message"]["content"] = json!([
                {"type":"text","text":"a".repeat(512 * 1024)},
                {"type":"text","text":"a".repeat(512 * 1024)}])
            }
            "metadata" => item["unknown_metadata"] = json!("x".repeat(180000)),
            "path" => {
                // Each entry fits; the full path budget does not. Forty-eight
                // modest buffers are needed, not an absurd multi-GiB input.
                for n in 0..48 {
                    let mut extra = boundary.clone();
                    extra["entry_id"] = json!(format!("extra-{n}"));
                    path.push(extra);
                }
            }
            "fork" => {
                item["message"]["content"][0]["text"] =
                    json!(format!("{}b", "a".repeat(96 * 1024 - 1)))
            }
            _ => unreachable!(),
        }
        path.push(item);
        stack.fixture.lock().unwrap().path = path;
        stack
            .invoke(delivery.clone(), "owner-one", Some(stack.handle.id()))
            .await;
        assert_eq!(
            stack.deps.store().await.extraction_plan(&key()).unwrap(),
            before,
            "{bad}"
        );
        assert!(stack.fixture.lock().unwrap().cursor.is_null());
        assert!(stack.fixture.lock().unwrap().queued.is_empty());
    }
    stack.fixture.lock().unwrap().path = vec![boundary];
    stack
        .invoke(delivery, "owner-one", Some(stack.handle.id()))
        .await;
    assert_eq!(
        Store::open(dir.path().into())
            .unwrap()
            .extraction_plan(&key())
            .unwrap(),
        before
    );
}

/// A later turn: its request and its stamped reply, under `new-bank`.
fn next_turn(stack: &Stack, mut path: Vec<Value>) -> memory::hooks::ExtractJobInput {
    path.push(json!({"entry_id":"next-request","message":{"role":"user","timestamp":1700000001000u64,"content":[{"type":"text","text":"The next request belongs only to the next bank."}]}}));
    path.push(json!({"entry_id":"next-end","message":{"role":"assistant","timestamp":1700000002000u64,"content":[{"type":"text","text":"Next answer."}]},"origin":{"turn_id":"next-turn","memory_extraction":true,"memory_bank":"new-bank"}}));
    stack.fixture.lock().unwrap().path = path;
    memory::hooks::ExtractJobInput {
        session_id: "synthetic-session".into(),
        turn_id: Some("next-turn".into()),
        bank: Some("new-bank".into()),
        until: Some("next-end".into()),
        ..Default::default()
    }
}

async fn enable_extraction(stack: &Stack) {
    let mut cfg = (*stack.deps.config().await).clone();
    cfg.extraction_enabled = true;
    cfg.extraction_model = "fixture-only".into();
    *stack.deps.config.write().await = Arc::new(cfg);
}

#[tokio::test]
async fn a_cancelled_request_after_a_layer_rotation_closes_unextracted() {
    let dir = tempfile::tempdir().unwrap();
    let stack = Stack::new(dir.path()).await;
    let mut delivery = input("cancelled");
    let a = &mut delivery["recall_context"]["extraction_history"]["rows"][0]["attestation"];
    a["terminal"]["status"] = json!("cancelled");
    a["layer"] = json!("public");
    stack
        .invoke(delivery, "owner-one", Some(stack.handle.id()))
        .await;
    assert!(stack
        .deps
        .store()
        .await
        .extraction_plan(&key())
        .unwrap()
        .is_some());
    *stack.deps.store.write().await = Arc::new(Store::open(dir.path().into()).unwrap());
    enable_extraction(&stack).await;
    let mut cfg = (*stack.deps.config().await).clone();
    cfg.default_bank = "rotated".into();
    *stack.deps.config.write().await = Arc::new(cfg);
    let job = memory::hooks::ExtractJobInput {
        session_id: "synthetic-session".into(),
        turn_id: Some("old-cancelled".into()),
        bank: Some("old-bank".into()),
        derived: true,
        until: Some("opaque-entry".into()),
        ..Default::default()
    };
    // The frozen layer is gone: the cursor passes the request with no model
    // call and nothing is written to any bank.
    memory::extract::try_run(&stack.deps, &job).await.unwrap();
    assert_eq!(
        stack.fixture.lock().unwrap().cursor,
        json!({"last_entry_id":"opaque-entry"})
    );
    assert!(stack.fixture.lock().unwrap().prompts.is_empty());
    assert!(!dir.path().join("old-bank/memories.jsonl").exists());
    assert!(!dir.path().join("rotated/memories.jsonl").exists());
    // The next turn extracts as usual.
    let next = next_turn(&stack, vec![source()]);
    memory::extract::try_run(&stack.deps, &next).await.unwrap();
    assert_eq!(stack.fixture.lock().unwrap().prompts.len(), 1);
    assert_eq!(
        stack.fixture.lock().unwrap().cursor,
        json!({"last_entry_id":"next-end"})
    );
}

#[tokio::test]
async fn an_unfinished_turn_never_authorizes_recovery_nor_stops_the_next_turn() {
    let dir = tempfile::tempdir().unwrap();
    let stack = Stack::new(dir.path()).await;
    enable_extraction(&stack).await;
    let mut unfinished = input("unfinished");
    unfinished["recall_context"]["extraction_history"]["rows"][0]["attestation"]["terminal"]
        ["exchange_finished"] = json!(false);
    stack
        .invoke(unfinished, "owner-one", Some(stack.handle.id()))
        .await;
    // It adds no authority, so the hook never imports it.
    assert_eq!(
        stack.deps.store().await.extraction_plan(&key()).unwrap(),
        None
    );
    assert!(stack.fixture.lock().unwrap().queued.is_empty());
    let job = memory::hooks::ExtractJobInput {
        session_id: "synthetic-session".into(),
        turn_id: Some("old-unfinished".into()),
        bank: Some("old-bank".into()),
        until: Some("opaque-entry".into()),
        ..Default::default()
    };
    // No authority: the request waits for its turn's reply, as without the
    // ledger, and nothing fails.
    memory::extract::try_run(&stack.deps, &job).await.unwrap();
    assert!(stack.fixture.lock().unwrap().prompts.is_empty());
    assert!(stack.fixture.lock().unwrap().cursor.is_null());
    // The next turn's request opens its own run: the unfinished request
    // closes unextracted and the cursor moves past both.
    let next = next_turn(&stack, vec![source()]);
    memory::extract::try_run(&stack.deps, &next).await.unwrap();
    assert_eq!(stack.fixture.lock().unwrap().prompts.len(), 1);
    assert!(!dir.path().join("old-bank/memories.jsonl").exists());
    assert_eq!(
        stack.fixture.lock().unwrap().cursor,
        json!({"last_entry_id":"next-end"})
    );
}

#[tokio::test]
async fn a_failed_turn_with_an_assistant_step_extracts_and_the_next_turn_follows() {
    let dir = tempfile::tempdir().unwrap();
    let stack = Stack::new(dir.path()).await;
    enable_extraction(&stack).await;
    let text = "The user prefers copper instruments.";
    let request = json!({"entry_id":"opaque-entry","message":{"role":"user","timestamp":1700000000000u64,"content":[{"type":"text","text":text}]}});
    let step = json!({"entry_id":"failed-step","message":{"role":"assistant","timestamp":1700000000500u64,"content":[{"type":"text","text":"Checking the instruments."}]},"origin":{"turn_id":"old-now","memory_extraction":true,"memory_bank":"old-bank"}});
    stack.fixture.lock().unwrap().path = vec![request.clone(), step.clone()];
    let mut delivery = input("now");
    delivery["recall_context"]["extraction_history"]["rows"][0]["attestation"]["text_sha256"] =
        json!("75a1e6098f1a3f42d47fb37d2f317b2b836b7798edbd06b88d37c0114b6ae438");
    stack
        .invoke(delivery, "owner-one", Some(stack.handle.id()))
        .await;
    assert!(stack
        .deps
        .store()
        .await
        .extraction_plan(&key())
        .unwrap()
        .is_some());
    // The row is no cut (an assistant step follows): the stamped step takes
    // the request into its own turn's bank, then the next turn runs.
    let next = next_turn(&stack, vec![request, step]);
    memory::extract::try_run(&stack.deps, &next).await.unwrap();
    let prompts = stack.fixture.lock().unwrap().prompts.clone();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[0].contains(text));
    assert!(!prompts[1].contains(text));
    assert!(dir.path().join("old-bank/memories.jsonl").exists());
    assert_eq!(
        stack.fixture.lock().unwrap().cursor,
        json!({"last_entry_id":"next-end"})
    );
}

#[tokio::test]
async fn a_default_bank_turn_goes_from_hook_to_job_and_is_extracted() {
    let dir = tempfile::tempdir().unwrap();
    let stack = Stack::new(dir.path()).await;
    enable_extraction(&stack).await;
    // No `fs_scope.root` and no agent profile: the resolver's own default
    // bank, which the stamp carries as its name alone.
    let hook = stack
        .invoke(
            json!({"session_id":"synthetic-session","turn_id":"default-turn"}),
            "owner-one",
            Some(stack.handle.id()),
        )
        .await;
    let mut origin = hook["result"]["annotations"].clone();
    assert_eq!(origin["memory_bank"], "main");
    origin["turn_id"] = json!("default-turn");
    let text = "The user prefers copper instruments.";
    stack.fixture.lock().unwrap().path = vec![
        json!({"entry_id":"default-request","message":{"role":"user","timestamp":1700000000000u64,"content":[{"type":"text","text":text}]}}),
        json!({"entry_id":"default-end","message":{"role":"assistant","timestamp":1700000000500u64,"content":[{"type":"text","text":"Noted."}]},"origin":origin}),
    ];
    let completed =
        serde_json::from_value(json!({"session_id":"synthetic-session","turn_id":"default-turn"}))
            .unwrap();
    memory::hooks::turn_completed(&stack.deps, completed)
        .await
        .unwrap();
    let queued = stack.fixture.lock().unwrap().queued.clone();
    assert_eq!(queued.len(), 1);
    let job: memory::hooks::ExtractJobInput = serde_json::from_value(queued[0].clone()).unwrap();
    assert_eq!(
        (job.bank.as_deref(), job.derived, job.until.as_deref()),
        (Some("main"), true, Some("default-end"))
    );
    memory::extract::try_run(&stack.deps, &job).await.unwrap();
    let prompts = stack.fixture.lock().unwrap().prompts.clone();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].contains(text));
    assert!(dir.path().join("main/memories.jsonl").exists());
    assert_eq!(
        stack.fixture.lock().unwrap().cursor,
        json!({"last_entry_id":"default-end"})
    );
}

#[tokio::test]
async fn a_failed_request_into_a_trashed_default_bank_is_still_recovered() {
    let dir = tempfile::tempdir().unwrap();
    let stack = Stack::new(dir.path()).await;
    enable_extraction(&stack).await;
    let mut cfg = (*stack.deps.config().await).clone();
    cfg.default_bank = "old-bank".into();
    *stack.deps.config.write().await = Arc::new(cfg);
    let store = stack.deps.store().await;
    store.ensure_bank("old-bank", None).await.unwrap();
    store.trash_bank("old-bank", "old-bank").await.unwrap();
    drop(store);
    // The default materializes on use: its request is queued like any other.
    stack
        .invoke(input("now"), "owner-one", Some(stack.handle.id()))
        .await;
    let queued = stack.fixture.lock().unwrap().queued.clone();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0]["bank"], "old-bank");
}
