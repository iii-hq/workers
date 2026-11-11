//! Delivery regressions over an isolated SDK transport, never a live engine.
use super::*;
use crate::hooks::recall::{resolve, SCOPE};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Data {
    state: BTreeMap<(String, String), Value>,
    messages: BTreeMap<String, Vec<Value>>,
    calls: Vec<(String, Value)>,
}
impl Data {
    fn get(&self, scope: &str, key: &str) -> Value {
        self.state
            .get(&(scope.into(), key.into()))
            .cloned()
            .unwrap_or(Value::Null)
    }
    fn rpc(&mut self, function: &str, p: &Value, action: &Value) -> Result<Value, String> {
        self.calls.push((function.into(), p.clone()));
        if action["type"] == "enqueue" {
            return Ok(json!({"message_receipt_id":"inert"}));
        }
        let scope = p["scope"].as_str().unwrap_or_default();
        let key = p["key"].as_str().unwrap_or_default();
        let session = p["session_id"].as_str().unwrap_or_default();
        match function {
            "state::get" | "harness::state::get" => Ok(self.get(scope, key)),
            "state::set" => {
                if scope == SCOPE {
                    return Err("public receipt write denied".into());
                }
                self.state
                    .insert((scope.into(), key.into()), p["value"].clone());
                Ok(json!({}))
            }
            "state::delete" => {
                self.state.remove(&(scope.into(), key.into()));
                Ok(json!({}))
            }
            "state::list" => Ok(json!(self
                .state
                .iter()
                .filter(|((s, _), _)| s == scope)
                .map(|(_, v)| v)
                .collect::<Vec<_>>())),
            "harness::state::compare-and-set" => {
                let old = self.get(scope, key);
                let swapped = old == p["expected"];
                if swapped {
                    self.state
                        .insert((scope.into(), key.into()), p["value"].clone());
                }
                Ok(json!({"swapped":swapped,"current":old}))
            }
            "session::get" => Ok(json!({"meta":{"session_id":session,"metadata":{}}})),
            "session::ensure" => {
                Ok(json!({"created":false,"meta":{"session_id":session,"metadata":{}}}))
            }
            "session::set-meta" | "session::set-status" => Ok(json!({})),
            "session::messages" => {
                Ok(json!({"messages":self.messages.get(session).cloned().unwrap_or_default()}))
            }
            "session::append" => {
                let id = p["entry_id"].as_str().ok_or("entry id missing")?;
                let entries = self.messages.entry(session.into()).or_default();
                if !entries.iter().any(|e| e["entry_id"] == id) {
                    entries
                        .push(json!({"entry_id":id,"message":p["message"],"origin":p["origin"]}));
                }
                Ok(json!({"entry_id":id}))
            }
            "engine::workers::register" => Ok(json!({"success":true})),
            "engine::functions::list" => Ok(json!({"functions":[]})),
            "state::claim-namespace" => Ok(json!({"claimed":true})),
            _ => Err(format!("unexpected isolated delivery RPC {function}")),
        }
    }
}
struct Stack {
    deps: Deps,
    data: Arc<Mutex<Data>>,
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
        let data = Arc::new(Mutex::new(Data::default()));
        let shared = data.clone();
        let (ready, wait) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            socket.send(tokio_tungstenite::tungstenite::Message::Text(json!({"type":"workerregistered","worker_id":"delivery-offline","features":["observe.v1","observe.v2","local-dispatch.v1"]}).to_string().into())).await.unwrap();
            let _ = ready.send(());
            let mut revision = 0_u64;
            while let Some(Ok(frame)) = socket.next().await {
                let Ok(text) = frame.to_text() else { continue };
                let Ok(request) = serde_json::from_str::<Value>(text) else {
                    continue;
                };
                let reply = if request["type"] == "registerfunction" {
                    revision += 1;
                    json!({"type":"functionregistered","id":request["id"],"namespace":"default","revision":revision,"local_dispatch":true,"observers":[]})
                } else if request["type"] == "invokefunction" {
                    let function = request["function_id"].as_str().unwrap();
                    let result =
                        shared
                            .lock()
                            .unwrap()
                            .rpc(function, &request["data"], &request["action"]);
                    let mut reply = json!({"type":"invocationresult","invocation_id":request["invocation_id"],"function_id":function});
                    match result {
                        Ok(value) => reply["result"] = value,
                        Err(error) => {
                            reply["error"] = json!({"code":"offline_denied","message":error})
                        }
                    }
                    reply
                } else {
                    continue;
                };
                if socket
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        reply.to_string().into(),
                    ))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let iii = Arc::new(iii_sdk::register_worker(
            &url,
            iii_sdk::InitOptions::default(),
        ));
        wait.await.unwrap();
        let deps = Deps::new(
            iii.clone(),
            Arc::new(tokio::sync::RwLock::new(Arc::new(WorkerConfig {
                session_timeout_ms: 1000,
                ..Default::default()
            }))),
            crate::discovery::new_cell(),
            crate::skills::new_cell(),
            crate::events::TurnEvents::register(&iii),
            crate::hooks::HookRegistry::register(&iii),
        );
        Self { deps, data, server }
    }
    async fn reset(&self, session: &str, status: TurnStatus) {
        let mut record = crate::types::turn::tests::record();
        record.session_id = session.into();
        record.turn_id = format!("t-{session}");
        record.status = status;
        record.options.system_prompt = Some("Frozen delivery test prompt".into());
        record.options.system_prompt_ref = None;
        record.options.skill_context = None;
        record.options.functions = None;
        crate::state::put_turn(&self.deps.iii, &record, 1000)
            .await
            .unwrap();
    }
    async fn context(&self, session: &str) -> Value {
        let record = crate::state::get_turn(&self.deps.iii, session, 1000)
            .await
            .unwrap()
            .unwrap();
        let entries = self
            .deps
            .session()
            .await
            .messages_strict(session)
            .await
            .unwrap();
        resolve(&self.deps, &record, &entries).await.unwrap()
    }
    async fn drain(&self, session: &str) -> usize {
        crate::turn_loop::drain_queued(&self.deps, &self.deps.session().await, session)
            .await
            .unwrap()
    }
    fn receipt(&self, session: &str, id: &str) -> Value {
        self.data
            .lock()
            .unwrap()
            .get(SCOPE, session)
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["entry_id"] == id)
            .unwrap()
            .clone()
    }
}
fn request(session: &str, text: &str, key: Option<&str>, prior: Option<&str>) -> SendRequest {
    serde_json::from_value(json!({"session_id":session,"message":text,"idempotency_key":key,"options":{"continuation_entry_id":prior}})).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn direct_delivery_keeps_one_transcript_receipt_and_source_identity() {
    let stack = Stack::new().await;
    stack.reset("direct", TurnStatus::Completed).await;
    handle(
        &stack.deps,
        request("direct", "First neutral request", None, None),
    )
    .await
    .unwrap();
    let context = stack.context("direct").await;
    let id = context["source"]["entry_id"].as_str().unwrap();
    assert!(id.starts_with("e_") && !id.starts_with("e_q_"), "{id}");
    assert_eq!(context["source"]["kind"], "direct");
    assert!(context["continuation"].is_null());
    assert_eq!(stack.receipt("direct", id)["query"]["entry_id"], id);
    assert!(stack.data.lock().unwrap().messages["direct"][0]["origin"].is_null());
    stack.reset("direct", TurnStatus::Completed).await;
    handle(
        &stack.deps,
        request("direct", "Next unrelated request", None, None),
    )
    .await
    .unwrap();
    let next = stack.context("direct").await;
    assert_ne!(next["source"]["entry_id"], id);
    assert!(next["continuation"].is_null());
    assert_eq!(stack.data.lock().unwrap().messages["direct"].len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_delivery_drain_and_subsequent_send_keep_distinct_stable_ids() {
    let stack = Stack::new().await;
    stack.reset("queue", TurnStatus::Running).await;
    let out = handle(
        &stack.deps,
        request("queue", "Parked neutral request", None, None),
    )
    .await
    .unwrap();
    assert_eq!(out.queued, Some(true));
    let rows = crate::state::list_queued(&stack.deps.iii, "queue", 1000)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert!(row.id.starts_with("q_"));
    assert_eq!(row.entry_id, ids::queued_entry_id(&row.id));
    assert!(row.entry_id.starts_with("e_q_"));
    assert_eq!(
        stack.receipt("queue", &row.entry_id)["query"]["entry_id"],
        row.entry_id
    );
    assert!(!stack.data.lock().unwrap().messages.contains_key("queue"));
    assert_eq!(stack.drain("queue").await, 1);
    // Simulate append success followed by lost queue deletion/recovery.
    crate::state::enqueue_message(&stack.deps.iii, row, 1000)
        .await
        .unwrap();
    assert_eq!(stack.drain("queue").await, 1);
    assert_eq!(stack.drain("queue").await, 0);
    assert_eq!(stack.data.lock().unwrap().messages["queue"].len(), 1);
    assert_eq!(
        stack.context("queue").await["source"],
        json!({"kind":"direct","entry_id":row.entry_id})
    );
    stack.reset("queue", TurnStatus::Completed).await;
    handle(
        &stack.deps,
        request("queue", "Continue this request", None, Some(&row.entry_id)),
    )
    .await
    .unwrap();
    let next = stack.context("queue").await;
    assert_ne!(next["source"]["entry_id"], row.entry_id);
    assert!(!next["source"]["entry_id"]
        .as_str()
        .unwrap()
        .starts_with("e_q_"));
    assert_eq!(next["continuation"]["entry_id"], row.entry_id);
    assert_eq!(next["continuation"]["text"], "Parked neutral request");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idempotency_keys_keep_legacy_ids_and_cannot_rewrite_continuation_receipts() {
    let stack = Stack::new().await;
    for (session, status) in [
        ("key-direct", TurnStatus::Completed),
        ("key-queue", TurnStatus::Running),
    ] {
        stack.reset(session, status).await;
        let key = format!("{session}/request");
        let id = ids::idem_user_entry_id(&key);
        handle(
            &stack.deps,
            request(session, "Original keyed request", Some(&key), None),
        )
        .await
        .unwrap();
        if status == TurnStatus::Running {
            let rows = crate::state::list_queued(&stack.deps.iii, session, 1000)
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].id, id);
            assert_eq!(rows[0].entry_id, id);
            assert_eq!(stack.receipt(session, &id)["query"]["entry_id"], id);
            assert_eq!(stack.drain(session).await, 1);
        }
        let receipt = stack.receipt(session, &id);
        assert_eq!(stack.context(session).await["source"]["entry_id"], id);
        assert_eq!(
            stack
                .data
                .lock()
                .unwrap()
                .get(crate::state::IDEM_SCOPE, &key)["entry_id"],
            id
        );
        // Existing keys intentionally deduplicate before validating a new body/reference.
        let duplicate = handle(
            &stack.deps,
            request(session, "Changed retry", Some(&key), Some(&id)),
        )
        .await
        .unwrap();
        assert_eq!(duplicate.deduplicated, Some(true));
        assert_eq!(stack.receipt(session, &id), receipt);
        assert_eq!(stack.data.lock().unwrap().messages[session].len(), 1);
        stack.reset(session, TurnStatus::Completed).await;
        handle(
            &stack.deps,
            request(
                session,
                "New keyed continuation",
                Some(&format!("{key}-next")),
                Some(&id),
            ),
        )
        .await
        .unwrap();
        assert_eq!(stack.context(session).await["continuation"]["entry_id"], id);
        // Losing the public idempotency mapping must not mutate private provenance.
        stack
            .data
            .lock()
            .unwrap()
            .state
            .remove(&(crate::state::IDEM_SCOPE.into(), key.clone()));
        assert!(handle(
            &stack.deps,
            request(session, "Changed retry", Some(&key), None)
        )
        .await
        .is_err());
        assert!(handle(
            &stack.deps,
            request(session, "Original keyed request", Some(&key), Some(&id))
        )
        .await
        .is_err());
        assert_eq!(stack.receipt(session, &id), receipt);
        assert_eq!(stack.data.lock().unwrap().messages[session].len(), 2);
        // Identical retry after mapping loss appends idempotently under the original ID.
        handle(
            &stack.deps,
            request(session, "Original keyed request", Some(&key), None),
        )
        .await
        .unwrap();
        stack.drain(session).await;
        assert_eq!(stack.data.lock().unwrap().messages[session].len(), 2);
        assert_eq!(stack.receipt(session, &id), receipt);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn continuation_rejects_missing_mismatched_or_cross_channel_receipts_before_append() {
    let stack = Stack::new().await;
    for mode in [
        "missing",
        "changed-text",
        "other-session",
        "other-channel",
        "off-path",
    ] {
        let session = mode;
        stack.reset(session, TurnStatus::Completed).await;
        handle(
            &stack.deps,
            request(session, "Original neutral request", None, None),
        )
        .await
        .unwrap();
        let prior = stack.context(session).await["source"]["entry_id"]
            .as_str()
            .unwrap()
            .to_owned();
        stack.reset(session, TurnStatus::Completed).await;
        {
            let mut data = stack.data.lock().unwrap();
            match mode {
                "missing" => {
                    data.state.remove(&(SCOPE.into(), session.into()));
                }
                "changed-text" => {
                    data.messages.get_mut(session).unwrap()[0]["message"] =
                        json!(AgentMessage::user_text("Different neutral text"));
                }
                "other-session" => {
                    data.state.get_mut(&(SCOPE.into(), session.into())).unwrap()[0]["session_id"] =
                        json!("elsewhere");
                }
                "off-path" => {
                    data.messages.get_mut(session).unwrap().clear();
                }
                _ => {}
            }
        }
        if matches!(mode, "missing" | "changed-text" | "other-session") {
            assert_eq!(stack.context(session).await["source"]["kind"], "unknown");
        }
        let (before_messages, before_receipts, before_appends) = {
            let data = stack.data.lock().unwrap();
            (
                data.messages[session].clone(),
                data.get(SCOPE, session),
                data.calls
                    .iter()
                    .filter(|(f, _)| f == "session::append")
                    .count(),
            )
        };
        let req = request(session, "Continue", None, Some(&prior));
        let result = if mode == "other-channel" {
            handle_from_invoke(&stack.deps, req, "caller", false).await
        } else {
            handle(&stack.deps, req).await
        };
        assert!(result.is_err(), "{mode}");
        let data = stack.data.lock().unwrap();
        assert_eq!(data.messages[session], before_messages, "{mode}");
        assert_eq!(data.get(SCOPE, session), before_receipts, "{mode}");
        assert_eq!(
            data.calls
                .iter()
                .filter(|(f, _)| f == "session::append")
                .count(),
            before_appends,
            "{mode}"
        );
    }
}
