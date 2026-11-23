//! Delivery regressions over an isolated SDK transport, never a live engine.
use super::*;
use crate::hooks::recall::{SCOPE, resolve};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Data {
    state: BTreeMap<(String, String), Value>,
    messages: BTreeMap<String, Vec<Value>>,
    calls: Vec<(String, Value)>,
    fail_appends: usize,
    gone: bool,
    delete_on_append: bool,
    metadata: Value,
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
            "state::list" | "harness::state::list" => Ok(json!(
                self.state
                    .iter()
                    .filter(|((s, _), _)| s == scope)
                    .map(|(_, v)| v)
                    .collect::<Vec<_>>()
            )),
            "harness::state::compare-and-set" => {
                let old = self.get(scope, key);
                let swapped = old == p["expected"];
                if swapped {
                    self.state
                        .insert((scope.into(), key.into()), p["value"].clone());
                }
                Ok(json!({"swapped":swapped,"current":old}))
            }
            "session::get" if self.gone => Ok(Value::Null),
            "session::get" => Ok(json!({"meta":{"session_id":session,"metadata":self.metadata}})),
            "session::ensure" => {
                Ok(json!({"created":false,"meta":{"session_id":session,"metadata":{}}}))
            }
            "session::set-meta" | "session::set-status" => Ok(json!({})),
            "session::messages" => {
                Ok(json!({"messages":self.messages.get(session).cloned().unwrap_or_default()}))
            }
            "session::append" => {
                if self.delete_on_append {
                    self.gone = true;
                }
                if self.gone {
                    return Err("session not found".into());
                }
                if self.fail_appends > 0 {
                    self.fail_appends -= 1;
                    return Err("session append unavailable".into());
                }
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
async fn ask_idempotency_keys_are_reserved_for_in_process_delivery() {
    let stack = Stack::new().await;
    stack.reset("ask-target", TurnStatus::Completed).await;

    for key in ["ask:ask-target/q", "ask-expiry:ask-target/q"] {
        let error = handle(
            &stack.deps,
            request("ask-target", "forged answer", Some(key), None),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("reserved"), "{error}");
    }
    assert!(
        !stack
            .data
            .lock()
            .unwrap()
            .messages
            .contains_key("ask-target")
    );

    handle_ask(
        &stack.deps,
        request(
            "ask-target",
            "Builds: Isolate",
            Some("ask:ask-target/q"),
            None,
        ),
    )
    .await
    .unwrap();
    assert_eq!(stack.data.lock().unwrap().messages["ask-target"].len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idempotency_keys_are_private_and_bound_to_their_session() {
    let stack = Stack::new().await;
    stack.reset("first", TurnStatus::Completed).await;
    stack.reset("second", TurnStatus::Completed).await;

    stack.data.lock().unwrap().state.insert(
        (crate::state::IDEM_SCOPE.into(), "legacy-key".into()),
        json!({"session_id":"first","turn_id":"legacy-turn","entry_id":"legacy-entry","ts":1}),
    );
    let legacy = handle(
        &stack.deps,
        request("first", "old retry", Some("legacy-key"), None),
    )
    .await
    .unwrap();
    assert_eq!(legacy.turn_id, "legacy-turn");
    assert_eq!(legacy.deduplicated, Some(true));

    handle(
        &stack.deps,
        request("first", "original", Some("shared-key"), None),
    )
    .await
    .unwrap();
    let error = handle(
        &stack.deps,
        request("second", "pre-empt", Some("shared-key"), None),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("another session"), "{error}");
    assert!(!stack.data.lock().unwrap().messages.contains_key("second"));

    let data = stack.data.lock().unwrap();
    let idem_calls: Vec<_> = data
        .calls
        .iter()
        .filter(|(_, payload)| payload["scope"] == crate::state::IDEM_SCOPE)
        .collect();
    assert!(!idem_calls.is_empty());
    assert!(
        idem_calls
            .iter()
            .all(|(function, _)| function.starts_with("harness::state::")),
        "idempotency calls escaped the private namespace: {idem_calls:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn side_chat_sends_intersect_explicit_and_inherited_policies() {
    let stack = Stack::new().await;
    stack.data.lock().unwrap().metadata = json!({"side_chat_question":"original/q"});
    stack.reset("side", TurnStatus::Completed).await;
    let mut req = request("side", "Discuss the pending decision", None, None);
    req.session = Some(SessionInit {
        metadata: Some(json!({})),
        ..Default::default()
    });
    req.options.as_mut().unwrap().functions = Some(FunctionPolicy {
        allow: vec!["*".into()],
        deny: vec!["memory::recall".into()],
        expose: crate::types::turn::ExposeMode::Native,
    });
    handle(&stack.deps, req).await.unwrap();
    let mut turn = crate::state::get_turn(&stack.deps.iii, "side", 1000)
        .await
        .unwrap()
        .unwrap();
    let functions = turn.options.functions.as_ref().unwrap();
    let expected = WorkerConfig::default().side_chat_functions;
    assert_eq!(functions.allow, expected);
    assert_eq!(functions.expose, crate::types::turn::ExposeMode::Native);
    let policy = policy::CompiledPolicy::from(Some(functions));
    for id in [
        "kanban::ticket::update",
        "harness::spawn",
        "harness::ask::answer",
        "shell::exec",
        "memory::recall",
    ] {
        assert!(!policy.allows(id), "{id}");
    }
    let child = policy::subset_policy(Some(functions), None).unwrap();
    assert_eq!(child.allow, expected);

    turn.status = TurnStatus::Completed;
    crate::state::put_turn(&stack.deps.iii, &turn, 1000)
        .await
        .unwrap();
    let mut cfg = (*stack.deps.cfg().await).clone();
    cfg.side_chat_functions = vec!["session::get".into()];
    crate::configuration::apply_config(&stack.deps.config, cfg).await;
    handle(&stack.deps, request("side", "Another question", None, None))
        .await
        .unwrap();
    let turn = crate::state::get_turn(&stack.deps.iii, "side", 1000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(turn.options.functions.unwrap().allow, ["session::get"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_sends_keep_the_explicit_function_policy() {
    let stack = Stack::new().await;
    stack.reset("ordinary", TurnStatus::Completed).await;
    let mut req = request("ordinary", "Continue work", None, None);
    let functions = FunctionPolicy {
        allow: vec!["*".into()],
        deny: vec![],
        expose: Default::default(),
    };
    req.options.as_mut().unwrap().functions = Some(functions.clone());
    handle(&stack.deps, req).await.unwrap();
    let turn = crate::state::get_turn(&stack.deps.iii, "ordinary", 1000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(turn.options.functions, Some(functions));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_side_chat_sends_tighten_the_running_turn() {
    let stack = Stack::new().await;
    stack.reset("running-side", TurnStatus::Running).await;
    let mut turn = crate::state::get_turn(&stack.deps.iii, "running-side", 1000)
        .await
        .unwrap()
        .unwrap();
    turn.options.functions = Some(FunctionPolicy {
        allow: vec!["*".into()],
        ..Default::default()
    });
    crate::state::put_turn(&stack.deps.iii, &turn, 1000)
        .await
        .unwrap();
    stack.data.lock().unwrap().metadata = json!({"side_chat_question":"original/q"});

    let mut req = request("running-side", "Discuss the decision", None, None);
    req.options.as_mut().unwrap().functions = turn.options.functions;
    let out = handle(&stack.deps, req.clone()).await.unwrap();
    assert_eq!(out.queued, Some(true));
    let turn = crate::state::get_turn(&stack.deps.iii, "running-side", 1000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        turn.options.functions.unwrap().allow,
        WorkerConfig::default().side_chat_functions
    );

    let mut cfg = (*stack.deps.cfg().await).clone();
    cfg.side_chat_functions = vec!["session::get".into()];
    crate::configuration::apply_config(&stack.deps.config, cfg).await;
    handle(&stack.deps, req.clone()).await.unwrap();
    let turn = crate::state::get_turn(&stack.deps.iii, "running-side", 1000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(turn.options.functions.unwrap().allow, ["session::get"]);

    req.options.as_mut().unwrap().functions = Some(FunctionPolicy::default());
    handle(&stack.deps, req).await.unwrap();
    let turn = crate::state::get_turn(&stack.deps.iii, "running-side", 1000)
        .await
        .unwrap()
        .unwrap();
    assert!(turn.options.functions.unwrap().allow.is_empty());
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
    assert!(
        !next["source"]["entry_id"]
            .as_str()
            .unwrap()
            .starts_with("e_q_")
    );
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
        // Losing the idempotency mapping must not mutate private provenance.
        stack
            .data
            .lock()
            .unwrap()
            .state
            .remove(&(crate::state::IDEM_SCOPE.into(), key.clone()));
        assert!(
            handle(
                &stack.deps,
                request(session, "Original keyed request", Some(&key), Some(&id))
            )
            .await
            .is_err()
        );
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
        // As on main, a changed retry without a continuation still delivers,
        // onto the stored entry, and leaves the receipt untouched.
        stack
            .data
            .lock()
            .unwrap()
            .state
            .remove(&(crate::state::IDEM_SCOPE.into(), key.clone()));
        handle(
            &stack.deps,
            request(session, "Changed retry", Some(&key), None),
        )
        .await
        .unwrap();
        stack.drain(session).await;
        assert_eq!(stack.data.lock().unwrap().messages[session].len(), 2);
        assert_eq!(stack.receipt(session, &id), receipt);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_idempotency_key_over_the_receipt_bound_still_delivers_without_recall_authority() {
    let stack = Stack::new().await;
    for (session, status) in [
        ("long-direct", TurnStatus::Completed),
        ("long-queue", TurnStatus::Running),
    ] {
        let key = format!("{session}/{}", "k".repeat(300));
        stack.reset(session, status).await;
        handle(
            &stack.deps,
            request(session, "Keyed request", Some(&key), None),
        )
        .await
        .unwrap();
        if status == TurnStatus::Running {
            assert_eq!(stack.drain(session).await, 1);
        }
        let id = ids::idem_user_entry_id(&key);
        assert_eq!(stack.data.lock().unwrap().messages[session].len(), 1);
        assert!(stack.data.lock().unwrap().get(SCOPE, session).is_null());
        let context = stack.context(session).await;
        assert_eq!(context["source"]["entry_id"], id);
        assert_eq!(context["source"]["kind"], "unknown");
        // A continuation still needs a receipt, so the bound stays an error there.
        assert!(
            handle(
                &stack.deps,
                request(session, "Continue", Some(&format!("{key}-next")), Some(&id))
            )
            .await
            .is_err()
        );
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refired_notice_with_new_text_delivers_after_its_first_append_failed() {
    let stack = Stack::new().await;
    let session = "condfail";
    // Same stable entry id and origin as the condition-failure notice.
    let id = "e_condfail_sub_1";
    let origin = json!({"notification": true, "binding": "sub_1"});
    stack.reset(session, TurnStatus::Completed).await;
    stack.data.lock().unwrap().fail_appends = 1;
    let notice = |reason: &str| {
        AgentMessage::user_text(format!(
            "[notification] binding sub_1 fired but was NOT delivered: {reason}."
        ))
    };
    assert!(
        inject(
            &stack.deps,
            session,
            notice("condition timed out"),
            Some(id),
            Some(&origin)
        )
        .await
        .is_err()
    );
    // The receipt landed before the failed append and certifies the old text.
    assert_eq!(stack.receipt(session, id)["channel"], "notification");
    inject(
        &stack.deps,
        session,
        notice("condition function not found"),
        Some(id),
        Some(&origin),
    )
    .await
    .unwrap();
    let messages = stack.data.lock().unwrap().messages[session].clone();
    assert_eq!(messages.len(), 1);
    assert!(
        messages[0]["message"]
            .to_string()
            .contains("condition function not found")
    );
    let context = stack.context(session).await;
    assert_eq!(context["source"]["entry_id"], id);
    assert_eq!(context["source"]["kind"], "unknown");
}

async fn send_as(
    stack: &Stack,
    invoked: bool,
    req: SendRequest,
) -> Result<SendResponse, HarnessError> {
    if invoked {
        handle_from_invoke(&stack.deps, req, "caller", false).await
    } else {
        handle(&stack.deps, req).await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_keyed_resend_with_changed_text_lands_on_the_stored_entry_unless_it_continues() {
    let stack = Stack::new().await;
    for (session, invoked, kind) in [
        ("resend-direct", false, "direct"),
        ("resend-invoked", true, "invoked-agent"),
    ] {
        stack.reset(session, TurnStatus::Completed).await;
        send_as(
            &stack,
            invoked,
            request(session, "Earlier request", None, None),
        )
        .await
        .unwrap();
        let prior = stack.context(session).await["source"]["entry_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let key = format!("{session}/request");
        let id = ids::idem_user_entry_id(&key);
        stack.reset(session, TurnStatus::Completed).await;
        send_as(
            &stack,
            invoked,
            request(session, "Original keyed request", Some(&key), None),
        )
        .await
        .unwrap();
        let source = stack.context(session).await["source"].clone();
        assert_eq!(source, json!({"kind":kind,"entry_id":id}));
        let receipts = stack.data.lock().unwrap().get(SCOPE, session);
        // The idempotency mapping expired, so the retry reaches delivery again.
        stack
            .data
            .lock()
            .unwrap()
            .state
            .remove(&(crate::state::IDEM_SCOPE.into(), key.clone()));
        stack.reset(session, TurnStatus::Completed).await;
        let error = send_as(
            &stack,
            invoked,
            request(session, "Changed retry", Some(&key), Some(&prior)),
        )
        .await
        .unwrap_err();
        assert!(
            error.to_string().contains("different provenance"),
            "{error}"
        );
        send_as(
            &stack,
            invoked,
            request(session, "Changed retry", Some(&key), None),
        )
        .await
        .unwrap();
        // As on main: the session manager keeps the stored entry, so the
        // untouched receipt still certifies what the transcript holds.
        let messages = stack.data.lock().unwrap().messages[session].clone();
        assert_eq!(messages.len(), 2, "{kind}");
        assert_eq!(messages[1]["entry_id"], id);
        assert!(
            messages[1]["message"]
                .to_string()
                .contains("Original keyed request")
        );
        assert_eq!(stack.data.lock().unwrap().get(SCOPE, session), receipts);
        assert_eq!(stack.context(session).await["source"], source);
    }
}

/// A send with no options into a session whose turn failed on a transient
/// provider error (what the workflow worker sends to continue a read-only node)
/// seeds a new turn in the same session: same identity, a clean failure state,
/// and the transcript the failed turn left, with the continue appended after it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_send_into_a_failed_session_continues_it_with_the_same_identity() {
    let stack = Stack::new().await;
    let mut failed = crate::types::turn::tests::record();
    failed.session_id = "failed".into();
    failed.turn_id = "t-failed".into();
    failed.status = TurnStatus::Failed;
    failed.result_error = Some("The provider temporarily failed.".into());
    failed.failure = Some(crate::types::turn::TurnFailure {
        code: "router/provider_transient".into(),
        retryable: true,
    });
    failed.transient_resumes = 3;
    failed.options.provider = Some("anthropic".into());
    failed.options.system_prompt = Some("Workflow verifier profile".into());
    failed.options.agent = Some(crate::types::turn::AgentIdentity {
        id: "workflow-verifier".into(),
        name: None,
        icon: None,
        color: None,
    });
    failed.options.functions = Some(FunctionPolicy {
        allow: vec!["shell::exec".into(), "workflow-result::submit".into()],
        ..Default::default()
    });
    failed.options.metadata = Some(crate::types::turn::fs_scope_metadata("/work/tree"));
    crate::state::put_turn(&stack.deps.iii, &failed, 1000)
        .await
        .unwrap();
    // The failed turn ended on the harness's own resume nudge, so the continue
    // makes two user messages in a row.
    let earlier: Vec<Value> = ["Refute the design.", "Resume from the transcript."]
        .iter()
        .enumerate()
        .map(|(i, text)| json!({"entry_id": format!("e_{i}"), "message": AgentMessage::user_text(*text)}))
        .collect();
    stack
        .data
        .lock()
        .unwrap()
        .messages
        .insert("failed".into(), earlier.clone());

    let out = handle(
        &stack.deps,
        serde_json::from_value(json!({
            "session_id": "failed",
            "idempotency_key": "failed#transient-retry",
            "message": "Continue from where you stopped."
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_ne!(out.turn_id, "t-failed");
    assert!(
        out.merged != Some(true) && out.queued != Some(true),
        "{out:?}"
    );

    let next = crate::state::get_turn(&stack.deps.iii, "failed", 1000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.turn_id, out.turn_id);
    assert_eq!(next.status, TurnStatus::Running);
    assert_eq!(next.failure, None);
    assert_eq!(next.result_error, None);
    assert_eq!(next.transient_resumes, 0);
    assert_eq!(next.options.model, failed.options.model);
    assert_eq!(next.options.provider, failed.options.provider);
    assert_eq!(next.options.system_prompt, failed.options.system_prompt);
    assert_eq!(next.options.agent, failed.options.agent);
    assert_eq!(next.options.functions, failed.options.functions);
    assert_eq!(next.options.filesystem_root(), Some("/work/tree"));

    let transcript = stack
        .deps
        .session()
        .await
        .messages_strict("failed")
        .await
        .unwrap();
    assert_eq!(transcript.len(), 3);
    let messages = stack.data.lock().unwrap().messages["failed"].clone();
    assert_eq!(messages[..2], earlier[..]);
    assert!(
        messages[2]["message"]
            .to_string()
            .contains("Continue from where you stopped."),
        "{messages:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn existing_only_send_never_ensures_a_session() {
    let stack = Stack::new().await;
    stack.reset("s", TurnStatus::Completed).await;
    let mut req = request("s", "answer", Some("existing-only:s/q"), None);
    req.options.get_or_insert_default().require_existing_session = true;
    handle(&stack.deps, req.clone()).await.unwrap();
    assert_eq!(stack.data.lock().unwrap().messages["s"].len(), 1);
    stack.data.lock().unwrap().gone = true;
    let err = handle(&stack.deps, req).await.unwrap_err();
    assert!(
        err.to_string().contains("require_existing_session"),
        "{err}"
    );
    assert!(
        !stack
            .data
            .lock()
            .unwrap()
            .calls
            .iter()
            .any(|(id, _)| matches!(id.as_str(), "session::ensure" | "session::create"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_running_turn_cannot_queue_into_a_deleted_session() {
    let stack = Stack::new().await;
    stack.reset("s", TurnStatus::Running).await;
    stack.data.lock().unwrap().gone = true;
    let err = inject(
        &stack.deps,
        "s",
        AgentMessage::user_text("answer"),
        Some("answer-entry"),
        None,
    )
    .await
    .err()
    .expect("a missing session must refuse the queued delivery");
    assert!(err.to_string().contains("deleted session"), "{err}");
    assert!(
        crate::state::list_queued(&stack.deps.iii, "s", 1000)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ask_deleted_during_send_is_not_recreated_or_reported_delivered() {
    let stack = Stack::new().await;
    stack.reset("s", TurnStatus::Completed).await;
    let deps = stack.deps.clone();
    stack.deps.iii.register_function(
        crate::functions::SEND_ID,
        iii_sdk::RegisterFunction::new_async(move |req: SendRequest| {
            let deps = deps.clone();
            async move { handle(&deps, req).await.map_err(iii_sdk::Error::from) }
        }),
    );
    let req = serde_json::from_value(json!({"questions":[{"header":"Builds","question":"How?","options":[{"label":"Isolate"},{"label":"Allow-list"}]}]})).unwrap();
    crate::ask_store::accept(&stack.deps, "s", "t", "q", &req)
        .await
        .unwrap();
    stack.data.lock().unwrap().delete_on_append = true;
    let answer = serde_json::from_value(json!({"session_id":"s","question_id":"q", "answers":[{"header":"Builds","choice":"Isolate"}],"by":{"channel":"control-plane"}})).unwrap();
    let closed = crate::ask_store::answer(&stack.deps, answer).await.unwrap();
    assert_eq!(
        closed.outcome,
        Some(crate::ask_store::Outcome::NotDelivered)
    );
    assert!(
        !stack
            .data
            .lock()
            .unwrap()
            .calls
            .iter()
            .any(|(id, _)| matches!(id.as_str(), "session::ensure" | "session::create"))
    );
    assert!(
        stack
            .data
            .lock()
            .unwrap()
            .messages
            .get("s")
            .is_none_or(Vec::is_empty)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_deleted_purges_asks_and_queued_answers() {
    let stack = Stack::new().await;
    stack.reset("s", TurnStatus::Running).await;
    let req = serde_json::from_value(json!({"questions":[{"header":"Builds","question":"How?","options":[{"label":"Isolate"},{"label":"Allow-list"}]}]})).unwrap();
    crate::ask_store::accept(&stack.deps, "s", "t", "q", &req)
        .await
        .unwrap();
    handle_ask(
        &stack.deps,
        request("s", "queued answer", Some("ask:s/q"), None),
    )
    .await
    .unwrap();
    assert_eq!(
        crate::state::list_queued(&stack.deps.iii, "s", 1000)
            .await
            .unwrap()
            .len(),
        1
    );
    stack.data.lock().unwrap().gone = true;
    crate::functions::on_session_deleted::handle(
        &stack.deps,
        crate::functions::on_session_deleted::SessionDeletedEvent {
            session_id: "s".into(),
        },
    )
    .await
    .unwrap();
    assert!(
        crate::ask_store::list(&stack.deps, crate::ask_store::ListRequest::default())
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    assert!(
        crate::state::list_queued(&stack.deps.iii, "s", 1000)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        crate::state::get_turn(&stack.deps.iii, "s", 1000)
            .await
            .unwrap()
            .is_none()
    );
}
