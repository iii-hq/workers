//! Producer SDK loopback tests, including the real file-backed State adapter.
//! Transport/registry fixtures are not engine authentication or operator ACL proof.
use super::*;

use iii_state::adapters::{CompareAndSetOutcome, KvStoreAdapter, StateAdapter};
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Only the transport is synthetic: every state read/write/CAS below reaches
/// the unchanged State adapter/KvStore implementation. No caller is fabricated.
struct NativeStateStore {
    adapter: Arc<KvStoreAdapter>,
    ack_race: Mutex<Option<Arc<tokio::sync::Barrier>>>,
    ack_arrivals: AtomicUsize,
    ack_swaps: AtomicUsize,
    ack_conflicts: AtomicUsize,
}
impl NativeStateStore {
    fn open(path: &std::path::Path) -> Self {
        Self {
            adapter: Arc::new(KvStoreAdapter::new(Some(json!({
                "store_method":"file_based", "file_path":path,
                "save_interval_ms":600_000
            })))),
            ack_race: Mutex::new(None),
            ack_arrivals: AtomicUsize::new(0),
            ack_swaps: AtomicUsize::new(0),
            ack_conflicts: AtomicUsize::new(0),
        }
    }
    async fn get(&self, scope: &str, key: &str) -> Value {
        self.adapter
            .get(scope, key)
            .await
            .unwrap()
            .map(|v| v.as_ref().clone())
            .unwrap_or(Value::Null)
    }
    async fn rpc(&self, function: &str, p: &Value) -> Result<Value, String> {
        let scope = p["scope"].as_str().ok_or("missing scope")?;
        let key = p["key"].as_str().unwrap_or_default();
        // These are storage operations, NOT a reimplementation of State ACLs.
        match function {
            "state::get" | "harness::state::get" => Ok(self.get(scope, key).await),
            "state::set" => {
                self.adapter
                    .set(scope, key, p["value"].clone())
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(json!({}))
            }
            "state::delete" => {
                self.adapter
                    .delete(scope, key)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(json!({}))
            }
            "state::list" | "harness::state::list" => Ok(json!(self
                .adapter
                .list(scope)
                .await
                .map_err(|e| e.to_string())?)),
            "harness::state::compare-and-set" => {
                let is_ack = scope == SCOPE
                    && p["value"].as_array().is_some_and(|rows| {
                        rows.iter().any(|r| {
                            r["extraction"].as_array().is_some_and(|proofs| {
                                proofs.iter().any(|a| a.get("terminal").is_none())
                            })
                        })
                    });
                let barrier = self.ack_race.lock().unwrap().clone();
                if is_ack {
                    if let Some(barrier) = barrier {
                        // Both actual producer reads finish BEFORE either real CAS.
                        if self.ack_arrivals.fetch_add(1, Ordering::SeqCst) < 2 {
                            tokio::time::timeout(std::time::Duration::from_secs(5), barrier.wait())
                                .await
                                .map_err(|_| "CAS rendezvous timed out")?;
                        }
                    }
                }
                let result = self
                    .adapter
                    .compare_and_set(scope, key, p.get("expected"), p["value"].clone())
                    .await
                    .map_err(|e| e.to_string())?;
                if p["durable"] == true {
                    self.adapter.flush().await.map_err(|e| e.to_string())?;
                }
                Ok(match result {
                    CompareAndSetOutcome::Swapped { .. } => {
                        if is_ack {
                            self.ack_swaps.fetch_add(1, Ordering::SeqCst);
                        }
                        json!({"swapped":true,"durable":p["durable"]==true})
                    }
                    CompareAndSetOutcome::NotSwapped { current } => {
                        if is_ack {
                            self.ack_conflicts.fetch_add(1, Ordering::SeqCst);
                        }
                        json!({"swapped":false,"current":current,"durable":p["durable"]==true})
                    }
                })
            }
            _ => Err(format!("unexpected native state RPC {function}")),
        }
    }
}

struct NativeStack {
    stack: Stack,
    store: Arc<NativeStateStore>,
}
impl Deref for NativeStack {
    type Target = Stack;
    fn deref(&self) -> &Stack {
        &self.stack
    }
}
impl NativeStack {
    async fn open(path: &std::path::Path) -> Self {
        let store = Arc::new(NativeStateStore::open(path));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let data = Arc::new(Mutex::new(Data {
            memory_owner: true,
            ..Default::default()
        }));
        let shared = data.clone();
        let native = store.clone();
        let (ready, wait) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let (mut writer, mut reader) = socket.split();
            writer
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    json!({
                        "type":"workerregistered","worker_id":"native-producer-loopback",
                        "features":["observe.v1","observe.v2","local-dispatch.v1"]
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            let writer = Arc::new(tokio::sync::Mutex::new(writer));
            let mut pending = tokio::task::JoinSet::new();
            let mut revision = 0_u64;
            let _ = ready.send(());
            while let Some(Ok(frame)) = reader.next().await {
                let Ok(text) = frame.to_text() else { continue };
                let Ok(request) = serde_json::from_str::<Value>(text) else {
                    continue;
                };
                if request["type"] == "registerfunction" {
                    revision += 1;
                    let observers = if request["id"] == crate::hooks::ON_PRE_GENERATE_FN {
                        json!([{"id":"native-memory-capture","target":"memory::hook::pre-generate",
                            "args":{"timeout_ms":10000},"filters":{},
                            "capture":["input","output","caller"],"on":"success"}])
                    } else {
                        json!([])
                    };
                    writer.lock().await.send(tokio_tungstenite::tungstenite::Message::Text(json!({
                        "type":"functionregistered","id":request["id"],"namespace":"default",
                        "revision":revision,"local_dispatch":true,"observers":observers
                    }).to_string().into())).await.unwrap();
                    continue;
                }
                if request["type"] != "invokefunction" {
                    continue;
                }
                let native = native.clone();
                let shared = shared.clone();
                let writer = writer.clone();
                // Independent requests must execute concurrently, not in a
                // serial socket loop that conceals a get/set implementation.
                pending.spawn(async move {
                    let function = request["function_id"].as_str().unwrap();
                    let result = if function.starts_with("state::") && function != "state::claim-namespace"
                        || function.starts_with("harness::state::") {
                        shared.lock().unwrap().calls.push((function.into(), request["data"].clone()));
                        native.rpc(function, &request["data"]).await
                    } else {
                        shared.lock().unwrap().rpc(function, &request["data"], &request["action"])
                    };
                    let mut reply = json!({"type":"invocationresult","invocation_id":request["invocation_id"],"function_id":function});
                    match result {
                        Ok(value) => reply["result"] = value,
                        Err(error) => reply["error"] = json!({"code":"native_test_error","message":error}),
                    }
                    let _ = writer.lock().await.send(tokio_tungstenite::tungstenite::Message::Text(reply.to_string().into())).await;
                });
            }
            pending.shutdown().await;
        });
        let iii = Arc::new(iii_sdk::register_worker(
            &url,
            iii_sdk::InitOptions::default(),
        ));
        wait.await.unwrap();
        let cfg = Arc::new(tokio::sync::RwLock::new(Arc::new(
            crate::config::WorkerConfig {
                session_timeout_ms: 10000,
                ..Default::default()
            },
        )));
        let deps = Deps::new(
            iii.clone(),
            cfg,
            crate::discovery::new_cell(),
            crate::skills::new_cell(),
            crate::events::TurnEvents::register(&iii),
            crate::hooks::HookRegistry::register(&iii),
        );
        let stack = Stack { deps, data, server };
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while stack
                .deps
                .hooks
                .bindings(crate::hooks::HookPoint::PreGenerate)
                .is_empty()
            {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        Self { stack, store }
    }
    async fn close(self) {
        let Self { mut stack, store } = self;
        store.adapter.flush().await.unwrap();
        stack.deps.iii.shutdown();
        stack.server.abort();
        let _ = (&mut stack.server).await;
        drop(stack);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while Arc::strong_count(&store) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("all SDK bridge handlers dropped before reopen");
        assert_eq!(Arc::strong_count(&store.adapter), 1);
        let weak = Arc::downgrade(&store.adapter);
        drop(store);
        assert!(
            weak.upgrade().is_none(),
            "reopen cannot share the old adapter"
        );
    }
}

async fn native_terminal(
    stack: &NativeStack,
    record: &mut TurnRecord,
    status: crate::types::turn::TurnStatus,
) {
    record.status = status;
    finish_extraction(&stack.deps, record, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_producer_failed_preassistant_policy_history_survives_full_drop_reopen() {
    use crate::types::turn::TurnStatus;
    let dir = tempfile::tempdir().unwrap();
    let stack = NativeStack::open(dir.path()).await;
    let mut first = delivered(&stack, "s", "a", "Sensitive source body").await;
    acknowledge(&stack, &first, json!({"memory_extraction":true,"memory_bank":"bank-a","memory_layer":"derived","memory_agent":"agent-a"})).await;
    assert!(history(&stack, "s").await["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    native_terminal(&stack, &mut first, TurnStatus::Failed).await;
    let frozen = history(&stack, "s").await;
    assert_eq!(frozen["version"], 1);
    let row = &frozen["rows"][0];
    assert_eq!(row["source_kind"], "direct");
    assert_eq!(
        row["attestation"],
        json!({"version":1,"session_id":"s","turn_id":first.turn_id,
        "entry_id":"a","text_sha256":full_message_stamp(&AgentMessage::user_text("Sensitive source body")).unwrap(),
        "policy":"on","bank":"bank-a","layer":"derived","agent":"agent-a",
        "terminal":{"status":"failed","exchange_finished":true}})
    );
    acknowledge(
        &stack,
        &first,
        json!({"memory_extraction":false,"memory_bank":"replacement"}),
    )
    .await;
    native_terminal(&stack, &mut first, TurnStatus::Completed).await;
    assert_eq!(history(&stack, "s").await, frozen);
    for (id, annotations, policy, status) in [
        (
            "b",
            json!({"memory_extraction":false,"memory_bank":"bank-b"}),
            "off",
            TurnStatus::Cancelled,
        ),
        (
            "c",
            json!({"memory_bank":"bank-c"}),
            "unknown",
            TurnStatus::Completed,
        ),
        (
            "d",
            json!({"memory_extraction":true,"memory_bank":"bank-d"}),
            "on",
            TurnStatus::Failed,
        ),
    ] {
        let mut record = delivered(&stack, "s", id, "Later request").await;
        acknowledge(&stack, &record, annotations).await;
        native_terminal(&stack, &mut record, status).await;
        let h = history(&stack, "s").await;
        assert_eq!(
            h["rows"].as_array().unwrap().last().unwrap()["attestation"]["policy"],
            policy
        );
    }
    let mut no_ack = delivered(&stack, "s", "no-ack", "Failed before Memory answered").await;
    native_terminal(&stack, &mut no_ack, TurnStatus::Failed).await;
    let before = history(&stack, "s").await;
    assert_eq!(before["rows"].as_array().unwrap().len(), 4);
    assert_eq!(before["rows"][0]["attestation"]["bank"], "bank-a");
    assert_eq!(before["rows"][3]["attestation"]["bank"], "bank-d");
    let messages = stack.data.lock().unwrap().messages.clone();
    assert!(messages["s"].iter().all(|e| e["message"]["role"] == "user"));
    let ledger = stack.store.get(SCOPE, "s").await;
    assert!(!ledger.to_string().contains("Sensitive source body"));
    assert!(
        stack.data.lock().unwrap().state.is_empty(),
        "no map-backed state fallback"
    );
    stack.close().await;
    assert!(std::fs::read_dir(dir.path()).unwrap().any(|e| e
        .unwrap()
        .path()
        .extension()
        .is_some_and(|e| e == "bin")));
    let reopened = NativeStack::open(dir.path()).await;
    // The session service is a separate fixture; only transcript rows, never
    // a state image or Arc, are restored to it after a fresh disk-backed open.
    reopened.data.lock().unwrap().messages = messages;
    assert_eq!(history(&reopened, "s").await, before);
    let next = delivered(&reopened, "s", "next", "Resume after reopen").await;
    assert!(next
        .recall_context
        .as_ref()
        .unwrap()
        .get("extraction_history")
        .is_none());
    acknowledge(&reopened, &next, json!({"memory_extraction":false})).await;
    assert_eq!(
        reopened.data.lock().unwrap().hooks.last().unwrap()["recall_context"]["extraction_history"],
        before
    );
    let copied = reopened.data.lock().unwrap().messages["s"].clone();
    reopened
        .data
        .lock()
        .unwrap()
        .messages
        .insert("fork".into(), copied);
    reopened
        .store
        .adapter
        .set(SCOPE, "fork", ledger)
        .await
        .unwrap();
    let fork = delivered(&reopened, "fork", "fork-new", "Fork request").await;
    acknowledge(&reopened, &fork, json!({"memory_extraction":false})).await;
    assert!(
        reopened.data.lock().unwrap().hooks.last().unwrap()["recall_context"]["extraction_history"]
            ["rows"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    reopened.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_producer_concurrent_duplicate_cas_has_one_source_bound_immutable_ack() {
    use crate::types::turn::TurnStatus;
    let dir = tempfile::tempdir().unwrap();
    let stack = NativeStack::open(dir.path()).await;
    let mut first = delivered(&stack, "s", "first", "First source").await;
    stack.data.lock().unwrap().memory_response = json!({"decision":"continue","annotations":{"memory_extraction":true,"memory_bank":"first-bank"}});
    *stack.store.ack_race.lock().unwrap() = Some(Arc::new(tokio::sync::Barrier::new(2)));
    let (a, b) = tokio::join!(
        stack
            .deps
            .hooks
            .run_pre_generate(&first, 0, None, &[], 0, &[]),
        stack
            .deps
            .hooks
            .run_pre_generate(&first, 0, None, &[], 0, &[])
    );
    assert!(matches!(
        a,
        crate::hooks::runner::PreGenerateOutcome::Continue { .. }
    ));
    assert!(matches!(
        b,
        crate::hooks::runner::PreGenerateOutcome::Continue { .. }
    ));
    assert_eq!(stack.store.ack_arrivals.load(Ordering::SeqCst), 2);
    assert_eq!(stack.store.ack_swaps.load(Ordering::SeqCst), 1);
    assert_eq!(
        stack.store.ack_conflicts.load(Ordering::SeqCst),
        1,
        "a real stale CAS, not an injected conflict"
    );
    *stack.store.ack_race.lock().unwrap() = None;
    let mut later = delivered(&stack, "s", "later", "Other source").await;
    later.turn_id = first.turn_id.clone();
    later.recall_context.as_mut().unwrap()["turn_id"] = json!(first.turn_id);
    acknowledge(
        &stack,
        &later,
        json!({"memory_extraction":true,"memory_bank":"other-bank"}),
    )
    .await;
    native_terminal(&stack, &mut first, TurnStatus::Cancelled).await;
    let before = history(&stack, "s").await;
    assert_eq!(before["rows"].as_array().unwrap().len(), 1);
    assert_eq!(before["rows"][0]["attestation"]["entry_id"], "first");
    assert_eq!(before["rows"][0]["attestation"]["bank"], "first-bank");
    acknowledge(
        &stack,
        &first,
        json!({"memory_extraction":false,"memory_bank":"replacement"}),
    )
    .await;
    native_terminal(&stack, &mut first, TurnStatus::Completed).await;
    assert_eq!(history(&stack, "s").await, before);
    let ledger = stack.store.get(SCOPE, "s").await;
    assert_eq!(
        ledger
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["extraction"].as_array().map_or(0, Vec::len))
            .sum::<usize>(),
        1
    );
    assert!(stack
        .data
        .lock()
        .unwrap()
        .calls
        .iter()
        .filter(|(f, p)| f == "harness::state::compare-and-set" && p["scope"] == SCOPE)
        .all(|(_, p)| p["durable"] == true));
    stack.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_producer_full_digest_edit_truncation_and_legacy_fail_closed() {
    use crate::types::turn::TurnStatus;
    let dir = tempfile::tempdir().unwrap();
    let stack = NativeStack::open(dir.path()).await;
    let text = format!("{}original suffix", "a".repeat(3200));
    let mut record = delivered(&stack, "s", "a", &text).await;
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"bank-a"}),
    )
    .await;
    native_terminal(&stack, &mut record, TurnStatus::Failed).await;
    let original = stack.data.lock().unwrap().messages["s"].clone();
    assert_eq!(
        history(&stack, "s").await["rows"].as_array().unwrap().len(),
        1
    );
    let edited = AgentMessage::user_text(format!("{}changed suffix", "a".repeat(3200)));
    stack.data.lock().unwrap().messages.get_mut("s").unwrap()[0]["message"] = json!(edited);
    assert!(history(&stack, "s").await["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    edit_direct(
        &stack.deps,
        "s",
        "a",
        &AgentMessage::user_text(&text),
        &edited,
    )
    .await
    .unwrap();
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"wrong-bank"}),
    )
    .await;
    assert!(history(&stack, "s").await["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        stack.store.get(SCOPE, "s").await[0]["extraction"][0]["bank"],
        "bank-a"
    );
    stack
        .data
        .lock()
        .unwrap()
        .messages
        .insert("s".into(), Vec::new());
    assert!(history(&stack, "s").await["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    stack
        .data
        .lock()
        .unwrap()
        .messages
        .insert("s".into(), original);
    let legacy = json!({"session_id":"s","entry_id":"a","channel":"direct","query":null,"continuation":null});
    assert!(serde_json::from_value::<Receipt>(legacy.clone())
        .unwrap()
        .extraction
        .is_empty());
    stack
        .store
        .adapter
        .set(SCOPE, "s", json!([legacy]))
        .await
        .unwrap();
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"not-legacy"}),
    )
    .await;
    native_terminal(&stack, &mut record, TurnStatus::Failed).await;
    assert!(history(&stack, "s").await["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    let messages = stack.data.lock().unwrap().messages.clone();
    stack.close().await;
    let reopened = NativeStack::open(dir.path()).await;
    reopened.data.lock().unwrap().messages = messages;
    assert!(reopened.store.get(SCOPE, "s").await[0]
        .get("extraction")
        .is_none());
    assert!(history(&reopened, "s").await["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    let next = delivered(&reopened, "s", "after-legacy", "No policy inheritance").await;
    acknowledge(&reopened, &next, json!({"memory_extraction":false})).await;
    assert_eq!(
        reopened.data.lock().unwrap().hooks.last().unwrap()["recall_context"]["extraction_history"]
            ["rows"],
        json!([])
    );
    reopened.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_producer_retention_128_by_8_survives_reopen_without_bank_inheritance() {
    use crate::types::turn::TurnStatus;
    let dir = tempfile::tempdir().unwrap();
    let stack = NativeStack::open(dir.path()).await;
    let mut record = delivered(&stack, "s", "old", "Retained source").await;
    for n in 0..10 {
        record.turn_id = format!("retained-turn-{n}");
        record.recall_context.as_mut().unwrap()["turn_id"] = json!(record.turn_id);
        record.status = TurnStatus::Running;
        acknowledge(
            &stack,
            &record,
            json!({"memory_extraction":true,"memory_bank":format!("bank-{n}")}),
        )
        .await;
        native_terminal(&stack, &mut record, TurnStatus::Failed).await;
    }
    let h = history(&stack, "s").await;
    assert_eq!(
        h["retention"],
        json!({"delivery_entries":128,"turns_per_entry":8})
    );
    assert_eq!(h["rows"].as_array().unwrap().len(), 8);
    assert_eq!(h["rows"][0]["attestation"]["bank"], "bank-2");
    assert_eq!(h["rows"][7]["attestation"]["bank"], "bank-9");
    let retained_messages = stack.data.lock().unwrap().messages.clone();
    stack.close().await;
    let stack = NativeStack::open(dir.path()).await;
    stack.data.lock().unwrap().messages = retained_messages;
    assert_eq!(
        history(&stack, "s").await,
        h,
        "all eight producer attestations reopen from disk"
    );
    for n in 0..MAX_RECEIPTS {
        let _ = delivered(&stack, "s", &format!("entry-{n}"), "Later source").await;
    }
    let ledger = stack.store.get(SCOPE, "s").await;
    assert_eq!(ledger.as_array().unwrap().len(), 128);
    assert_eq!(ledger[0]["entry_id"], "entry-0");
    let h = history(&stack, "s").await;
    assert_eq!(h["missing"], "unknown");
    assert!(h["rows"].as_array().unwrap().is_empty());
    let messages = stack.data.lock().unwrap().messages.clone();
    stack.close().await;
    let reopened = NativeStack::open(dir.path()).await;
    reopened.data.lock().unwrap().messages = messages;
    assert_eq!(reopened.store.get(SCOPE, "s").await, ledger);
    assert_eq!(history(&reopened, "s").await, h);
    let next = delivered(&reopened, "s", "new", "No inherited bank").await;
    acknowledge(&reopened, &next, json!({})).await;
    assert_eq!(
        reopened.data.lock().unwrap().hooks.last().unwrap()["recall_context"]["extraction_history"]
            ["rows"],
        json!([])
    );
    reopened.close().await;
}

async fn delivered(stack: &Stack, session: &str, id: &str, text: &str) -> TurnRecord {
    stack
        .reset_turn(session, crate::types::turn::TurnStatus::Running)
        .await;
    let message = AgentMessage::user_text(text);
    record_delivery(
        &stack.deps,
        session,
        &Delivery {
            message: &message,
            entry_id: Some(id),
            origin: None,
            channel: DeliveryChannel::Direct,
            continuation_entry_id: None,
            lineage: &crate::functions::send::TurnLineage::default(),
            caller_holds_session_lock: false,
            skills_explicit: false,
        },
    )
    .await
    .unwrap();
    stack.data.lock().unwrap().messages.entry(session.into()).or_default()
        .push(json!({"entry_id":id,"message":message,"origin":{"memory_extraction":true,"memory_bank":"forged"}}));
    let mut record = stack.record(session).await;
    record.turn_id = format!("opaque-turn-{id}");
    let entries = stack
        .deps
        .session()
        .await
        .messages_strict(session)
        .await
        .unwrap();
    record.recall_context = resolve(&stack.deps, &record, &entries).await;
    record
}

async fn acknowledge(stack: &Stack, record: &TurnRecord, annotations: Value) {
    stack.data.lock().unwrap().memory_response =
        json!({"decision":"continue","annotations":annotations});
    let outcome = stack
        .deps
        .hooks
        .run_pre_generate(record, 0, None, &[], 0, &[])
        .await;
    assert!(matches!(
        outcome,
        crate::hooks::runner::PreGenerateOutcome::Continue { .. }
    ));
}

async fn history(stack: &Stack, session: &str) -> Value {
    let entries = stack
        .deps
        .session()
        .await
        .messages_strict(session)
        .await
        .unwrap();
    extraction_history(&stack.deps.iii, session, &entries, 1000)
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_preassistant_on_off_bank_changes_and_restart_retain_actual_ack() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let mut first = delivered(&stack, "s", "opaque-a", "Sensitive first request").await;
    acknowledge(&stack, &first, json!({"memory_extraction":true,"memory_bank":"bank-a","memory_layer":"derived","memory_agent":"agent-bank"})).await;
    assert!(
        history(&stack, "s").await["rows"]
            .as_array()
            .unwrap()
            .is_empty(),
        "no terminal boundary yet"
    );
    // Delivery redelivery must not conflict with newly attached producer proof.
    let original = AgentMessage::user_text("Sensitive first request");
    record_delivery(
        &stack.deps,
        "s",
        &Delivery {
            message: &original,
            entry_id: Some("opaque-a"),
            origin: None,
            channel: DeliveryChannel::Direct,
            continuation_entry_id: None,
            lineage: &crate::functions::send::TurnLineage::default(),
            caller_holds_session_lock: false,
            skills_explicit: false,
        },
    )
    .await
    .unwrap();
    first.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &first, true).await;
    // A repeated/out-of-order acknowledgement cannot attribute bank-b to a.
    acknowledge(
        &stack,
        &first,
        json!({"memory_extraction":true,"memory_bank":"bank-b"}),
    )
    .await;
    let mut second = delivered(&stack, "s", "opaque-b", "Second request").await;
    acknowledge(
        &stack,
        &second,
        json!({"memory_extraction":false,"memory_bank":"bank-b"}),
    )
    .await;
    second.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &second, true).await;
    finish_extraction(&stack.deps, &first, false).await;
    let h = history(&stack, "s").await;
    assert_eq!(h["rows"].as_array().unwrap().len(), 2);
    assert_eq!(h["rows"][0]["attestation"]["policy"], "on");
    assert_eq!(h["rows"][0]["attestation"]["bank"], "bank-a");
    assert_eq!(h["rows"][0]["attestation"]["agent"], "agent-bank");
    assert_eq!(
        h["rows"][0]["attestation"]["terminal"],
        json!({"status":"failed","exchange_finished":true})
    );
    assert_eq!(h["rows"][1]["attestation"]["policy"], "off");
    assert_eq!(h["rows"][1]["attestation"]["bank"], "bank-b");
    assert!(stack.data.lock().unwrap().messages["s"]
        .iter()
        .all(|e| e["message"]["role"] != "assistant"));
    let stored = stack.data.lock().unwrap().get(SCOPE, "s");
    assert!(!stored.to_string().contains("Sensitive first request"));
    let persisted_state = stack.data.lock().unwrap().state.clone();
    let persisted_messages = stack.data.lock().unwrap().messages.clone();
    drop(stack);
    // Recreate SDK/Harness state independently, retaining only the store image.
    // The loopback simulates persistence, not fsync/power-loss or engine ACLs.
    let restarted = Stack::with_target("memory::hook::pre-generate").await;
    restarted.data.lock().unwrap().state = persisted_state;
    restarted.data.lock().unwrap().messages = persisted_messages;
    assert_eq!(history(&restarted, "s").await, h);
    let current = delivered(&restarted, "s", "opaque-c", "Next turn").await;
    assert!(current
        .recall_context
        .as_ref()
        .unwrap()
        .get("extraction_history")
        .is_none());
    acknowledge(&restarted, &current, json!({"memory_extraction":false})).await;
    let replay = restarted.data.lock().unwrap().hooks.last().unwrap().clone();
    assert_eq!(
        replay["recall_context"]["extraction_history"]["rows"],
        h["rows"]
    );
    assert!(restarted
        .data
        .lock()
        .unwrap()
        .calls
        .iter()
        .filter(|(f, p)| f == "harness::state::compare-and-set" && p["scope"] == SCOPE)
        .all(|(_, p)| p["durable"] == true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prehook_failure_unknown_and_forged_metadata_never_promote() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let mut record = delivered(&stack, "s", "opaque-a", "No model response").await;
    record.options.metadata = Some(
        json!({"memory_extraction":true,"memory_bank":"forged","extraction_history":{"version":1,"policy":"on"}}),
    );
    record.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &record, true).await;
    let h = history(&stack, "s").await;
    assert_eq!(h["missing"], "unknown");
    assert!(h["rows"].as_array().unwrap().is_empty());
    record.status = crate::types::turn::TurnStatus::Running;
    acknowledge(&stack, &record, json!({"memory_bank":"not-enough"})).await;
    record.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &record, true).await;
    assert_eq!(
        history(&stack, "s").await["rows"][0]["attestation"]["policy"],
        "unknown"
    );
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"later-bank"}),
    )
    .await;
    assert_eq!(
        history(&stack, "s").await["rows"][0]["attestation"]["policy"],
        "unknown"
    );
    // The same annotations from a different target cannot produce an ack.
    let outsider = Stack::new().await;
    let mut other = delivered(&outsider, "s", "opaque-a", "No model response").await;
    acknowledge(
        &outsider,
        &other,
        json!({"memory_extraction":true,"memory_bank":"forged"}),
    )
    .await;
    other.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&outsider.deps, &other, true).await;
    assert!(history(&outsider, "s").await["rows"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_digest_edit_fork_active_path_and_retention_fail_closed() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let long = format!("{}original suffix", "a".repeat(3200));
    let mut record = delivered(&stack, "s", "opaque-a", &long).await;
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"a"}),
    )
    .await;
    record.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &record, true).await;
    assert_eq!(
        history(&stack, "s").await["rows"].as_array().unwrap().len(),
        1
    );
    let original = stack.data.lock().unwrap().messages["s"].clone();
    stack
        .data
        .lock()
        .unwrap()
        .messages
        .insert("fork".into(), original.clone());
    assert!(history(&stack, "fork").await["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    let edited = AgentMessage::user_text(format!("{}changed suffix", "a".repeat(3200)));
    stack.data.lock().unwrap().messages.get_mut("s").unwrap()[0]["message"] = json!(edited);
    assert!(
        history(&stack, "s").await["rows"]
            .as_array()
            .unwrap()
            .is_empty(),
        "full digest rejects suffix edits beyond recall prefix"
    );
    // Even a real subsequent Memory ack cannot bless an edited transcript.
    record.turn_id = "edited-turn".into();
    record.recall_context.as_mut().unwrap()["turn_id"] = json!(record.turn_id);
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"a"}),
    )
    .await;
    record.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &record, true).await;
    assert!(history(&stack, "s").await["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    stack
        .data
        .lock()
        .unwrap()
        .messages
        .insert("s".into(), Vec::new());
    assert!(history(&stack, "s").await["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    stack
        .data
        .lock()
        .unwrap()
        .messages
        .insert("s".into(), original);
    // An intervening source edit during a hook invocation fails the double read.
    let mut input = json!({"point":"pre_generate","session_id":"s","turn_id":"unseen",
        "recall_context":record.recall_context});
    input["recall_context"]["turn_id"] = json!("unseen");
    acknowledge_extraction(
        &stack.deps.iii,
        &input,
        &json!({"decision":"continue","annotations":{"memory_extraction":true,"memory_bank":"a"}}),
        Some("wrong digest"),
        1000,
    )
    .await
    .unwrap();
    assert!(!stack
        .data
        .lock()
        .unwrap()
        .get(SCOPE, "s")
        .to_string()
        .contains("unseen"));
    // Eviction is visible as absence/UNKNOWN, never as next bank inheritance.
    for n in 0..MAX_RECEIPTS {
        let id = format!("opaque-retained-{n}");
        let message = AgentMessage::user_text("new");
        record_delivery(
            &stack.deps,
            "s",
            &Delivery {
                message: &message,
                entry_id: Some(&id),
                origin: None,
                channel: DeliveryChannel::Direct,
                continuation_entry_id: None,
                lineage: &crate::functions::send::TurnLineage::default(),
                caller_holds_session_lock: false,
                skills_explicit: false,
            },
        )
        .await
        .unwrap();
    }
    let h = history(&stack, "s").await;
    assert_eq!(h["retention"]["delivery_entries"], 128);
    assert_eq!(h["missing"], "unknown");
    assert!(h["rows"].as_array().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_turns_on_same_source_bound_retention_and_freeze_unknown() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let mut record = delivered(&stack, "s", "opaque-a", "Repeated request").await;
    for n in 0..10 {
        record.turn_id = format!("turn-{n}");
        record.recall_context.as_mut().unwrap()["turn_id"] = json!(record.turn_id);
        record.status = crate::types::turn::TurnStatus::Running;
        acknowledge(
            &stack,
            &record,
            json!({"memory_extraction":true,"memory_bank":format!("bank-{n}")}),
        )
        .await;
        record.status = crate::types::turn::TurnStatus::Failed;
        finish_extraction(&stack.deps, &record, true).await;
    }
    let h = history(&stack, "s").await;
    assert_eq!(h["rows"].as_array().unwrap().len(), MAX_EXTRACTION_TURNS);
    assert_eq!(h["rows"][0]["attestation"]["turn_id"], "turn-2");
    assert_eq!(h["rows"][7]["attestation"]["bank"], "bank-9");
    // An envelope whose claimed session/turn does not match is not accepted.
    let forged = json!({"point":"pre_generate","session_id":"s","turn_id":"forged","recall_context":record.recall_context});
    acknowledge_extraction(
        &stack.deps.iii,
        &forged,
        &json!({"decision":"continue","annotations":{"memory_extraction":true,"memory_bank":"forged"}}),
        Some(&full_message_stamp(&AgentMessage::user_text("Repeated request")).unwrap()),
        1000,
    )
    .await
    .unwrap();
    assert_eq!(history(&stack, "s").await, h);
}

#[test]
fn acknowledgement_requires_explicit_policy_and_bank_without_guessing() {
    for (input, expected) in [
        (json!({}), "unknown"),
        (json!({"memory_extraction":true}), "unknown"),
        (
            json!({"memory_extraction":"on","memory_bank":"a"}),
            "unknown",
        ),
        (json!({"memory_extraction":true,"memory_bank":"a"}), "on"),
        (json!({"memory_extraction":false}), "off"),
    ] {
        assert_eq!(acknowledged_policy(&input).0, expected);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_hook_call_and_wrong_registry_actor_cannot_ack_or_project() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let mut record = delivered(&stack, "s", "opaque-a", "Real request").await;
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"a"}),
    )
    .await;
    record.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &record, true).await;
    let before = stack.data.lock().unwrap().get(SCOPE, "s");
    let forged = json!({"point":"pre_generate","session_id":"s","turn_id":"forged",
        "_caller_worker_id":"harness", "caller":{"worker_name":"harness"},
        "metadata":{"internal":true,"worker_name":"harness"},
        "recall_context":{"version":1,"session_id":"s","turn_id":"forged",
            "source":{"kind":"direct","entry_id":"opaque-a"},
            "extraction_history":{"version":1,"rows":[{"policy":"on"}]}},
        "generate":{"messages":[]}});
    let observers = stack.deps.iii.observers(crate::hooks::ON_PRE_GENERATE_FN);
    crate::hooks::runner::deliver(
        stack.deps.iii.clone(),
        crate::hooks::HookPoint::PreGenerate,
        iii_sdk::Call {
            function_id: crate::hooks::ON_PRE_GENERATE_FN.into(),
            input: forged.clone(),
            output: Some(forged),
            ..Default::default()
        },
        observers,
    )
    .await;
    assert_eq!(stack.data.lock().unwrap().get(SCOPE, "s"), before);
    assert!(
        stack.data.lock().unwrap().hooks.last().unwrap()["recall_context"]
            .get("extraction_history")
            .is_none()
    );

    // Engine-side owner mismatch cannot be repaired by JSON caller claims.
    stack.data.lock().unwrap().memory_owner = false;
    let mut other = delivered(&stack, "s", "opaque-b", "Different actor").await;
    other.options.metadata = Some(json!({"worker":"memory","memory_extraction":true}));
    acknowledge(
        &stack,
        &other,
        json!({"memory_extraction":true,"memory_bank":"forged"}),
    )
    .await;
    other.status = crate::types::turn::TurnStatus::Cancelled;
    finish_extraction(&stack.deps, &other, true).await;
    assert_eq!(
        history(&stack, "s").await["rows"].as_array().unwrap().len(),
        1
    );
    assert!(
        stack.data.lock().unwrap().hooks.last().unwrap()["recall_context"]
            .get("extraction_history")
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_duplicate_ack_cas_retry_and_terminal_are_immutable() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let mut record = delivered(&stack, "s", "opaque-a", "Concurrent request").await;
    stack.data.lock().unwrap().memory_response = json!({"decision":"continue",
        "annotations":{"memory_extraction":true,"memory_bank":"first"}});
    stack.data.lock().unwrap().extraction_cas_conflicts = 1;
    let (a, b) = tokio::join!(
        stack
            .deps
            .hooks
            .run_pre_generate(&record, 0, None, &[], 0, &[]),
        stack
            .deps
            .hooks
            .run_pre_generate(&record, 0, None, &[], 0, &[])
    );
    assert!(matches!(
        a,
        crate::hooks::runner::PreGenerateOutcome::Continue { .. }
    ));
    assert!(matches!(
        b,
        crate::hooks::runner::PreGenerateOutcome::Continue { .. }
    ));
    record.status = crate::types::turn::TurnStatus::Cancelled;
    finish_extraction(&stack.deps, &record, true).await;
    let before = history(&stack, "s").await;
    assert_eq!(before["rows"].as_array().unwrap().len(), 1);
    assert_eq!(
        before["rows"][0]["attestation"]["terminal"]["status"],
        "cancelled"
    );
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":false,"memory_bank":"second"}),
    )
    .await;
    record.status = crate::types::turn::TurnStatus::Completed;
    finish_extraction(&stack.deps, &record, false).await;
    assert_eq!(history(&stack, "s").await, before);
    let stored = stack.data.lock().unwrap().get(SCOPE, "s");
    assert_eq!(stored[0]["extraction"].as_array().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn edit_purge_legacy_and_malformed_rows_never_inherit_policy() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let mut record = delivered(&stack, "s", "opaque-a", "Original request").await;
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"a"}),
    )
    .await;
    record.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &record, true).await;
    let edited = AgentMessage::user_text("Edited request");
    edit_direct(
        &stack.deps,
        "s",
        "opaque-a",
        &AgentMessage::user_text("Original request"),
        &edited,
    )
    .await
    .unwrap();
    stack.data.lock().unwrap().messages.get_mut("s").unwrap()[0]["message"] = json!(edited);
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"b"}),
    )
    .await;
    assert!(history(&stack, "s").await["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        stack.data.lock().unwrap().get(SCOPE, "s")[0]["extraction"][0]["bank"],
        "a"
    );
    purge(&stack.deps, "s").await.unwrap();
    assert!(history(&stack, "s").await["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    let legacy = json!({"session_id":"s","entry_id":"opaque-a","channel":"direct",
        "query":null,"continuation":null});
    let receipt: Receipt = serde_json::from_value(legacy.clone()).unwrap();
    assert!(receipt.extraction.is_empty());
    stack
        .data
        .lock()
        .unwrap()
        .state
        .insert((SCOPE.into(), "s".into()), json!([legacy]));
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"c"}),
    )
    .await;
    assert!(history(&stack, "s").await["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    stack.data.lock().unwrap().state.insert(
        (SCOPE.into(), "s".into()),
        json!([{"extraction":"malformed"}]),
    );
    let entries = stack
        .deps
        .session()
        .await
        .messages_strict("s")
        .await
        .unwrap();
    assert!(extraction_history(&stack.deps.iii, "s", &entries, 1000)
        .await
        .is_err());
    // Generation still resolves its ordinary recall envelope, without proof.
    assert!(resolve(&stack.deps, &record, &entries)
        .await
        .unwrap()
        .get("extraction_history")
        .is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_ack_cannot_be_copied_to_a_later_source_in_the_same_turn() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let mut first = delivered(&stack, "s", "opaque-first", "First source").await;
    acknowledge(
        &stack,
        &first,
        json!({"memory_extraction":true,"memory_bank":"first"}),
    )
    .await;
    let mut second = delivered(&stack, "s", "opaque-second", "Second source").await;
    second.turn_id = first.turn_id.clone();
    second.recall_context.as_mut().unwrap()["turn_id"] = json!(first.turn_id);
    acknowledge(
        &stack,
        &second,
        json!({"memory_extraction":true,"memory_bank":"second"}),
    )
    .await;
    first.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &first, true).await;
    let h = history(&stack, "s").await;
    assert_eq!(h["rows"].as_array().unwrap().len(), 1);
    assert_eq!(h["rows"][0]["attestation"]["entry_id"], "opaque-first");
    assert_eq!(h["rows"][0]["attestation"]["bank"], "first");
    // Same historical ledger, but an unrelated hook target receives no proof.
    let outsider = Stack::new().await;
    outsider.data.lock().unwrap().state = stack.data.lock().unwrap().state.clone();
    outsider.data.lock().unwrap().messages = stack.data.lock().unwrap().messages.clone();
    acknowledge(
        &outsider,
        &second,
        json!({"memory_extraction":true,"memory_bank":"forged"}),
    )
    .await;
    assert!(
        outsider.data.lock().unwrap().hooks.last().unwrap()["recall_context"]
            .get("extraction_history")
            .is_none()
    );
}
