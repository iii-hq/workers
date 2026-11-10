// Isolated loopback SDK transport. No engine process, service, model or
// installed asset is started. Unexpected RPC fails closed in this boundary.
use super::*;
use futures_util::{SinkExt, StreamExt};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Data {
    state: BTreeMap<(String, String), Value>,
    messages: BTreeMap<String, Vec<Value>>,
    calls: Vec<(String, Value)>,
    hooks: Vec<Value>,
    memory_response: Value,
    memory_owner: bool,
    extraction_cas_conflicts: usize,
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
                    return Err("public write denied".into());
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
                if scope == SCOPE && self.extraction_cas_conflicts > 0 {
                    self.extraction_cas_conflicts -= 1;
                    return Ok(json!({"swapped":false,"current":old}));
                }
                let swapped = old == p["expected"];
                if swapped {
                    self.state
                        .insert((scope.into(), key.into()), p["value"].clone());
                }
                Ok(json!({"swapped":swapped,"current":old,"durable":p["durable"]==true}))
            }
            "session::create" => Ok(json!({"session_id":"created-agent-session"})),
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
            "engine::functions::list" => Ok(if self.memory_owner && p["worker"] == "memory" {
                json!({"functions":[{"function_id":"memory::hook::pre-generate"}]})
            } else {
                json!({"functions":[]})
            }),
            "state::claim-namespace" => Ok(json!({"claimed":true})),
            "memory::hook::pre-generate" => {
                self.hooks.push(p.clone());
                Ok(self.memory_response.clone())
            }
            "offline::capture" => {
                self.hooks.push(p.clone());
                Ok(if self.memory_response.is_null() {
                    json!({"decision":"continue"})
                } else {
                    self.memory_response.clone()
                })
            }
            _ => Err(format!("unexpected offline RPC {function}")),
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
        Self::with_target("offline::capture").await
    }
    async fn with_target(target: &'static str) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let data = Arc::new(Mutex::new(Data {
            memory_owner: target == "memory::hook::pre-generate",
            ..Default::default()
        }));
        let shared = data.clone();
        let (ready, wait) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let hello = json!({"type":"workerregistered","worker_id":"recall-offline",
                "features":["observe.v1","observe.v2","local-dispatch.v1"]});
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    hello.to_string().into(),
                ))
                .await
                .unwrap();
            let mut revision = 0_u64;
            let _ = ready.send(());
            while let Some(Ok(frame)) = socket.next().await {
                let Ok(text) = frame.to_text() else { continue };
                let Ok(request) = serde_json::from_str::<Value>(text) else {
                    continue;
                };
                if request["type"] == "registerfunction" {
                    revision += 1;
                    let observers = if request["id"] == crate::hooks::ON_PRE_GENERATE_FN {
                        json!([{"id":"recall-capture","target":target,
                            "args":{"timeout_ms":1000},"filters":{},
                            "capture":["input","output","caller"],"on":"success"}])
                    } else {
                        json!([])
                    };
                    let reply = json!({"type":"functionregistered","id":request["id"],
                        "namespace":"default","revision":revision,
                        "local_dispatch":true,"observers":observers});
                    socket
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            reply.to_string().into(),
                        ))
                        .await
                        .unwrap();
                    continue;
                }
                if request["type"] != "invokefunction" {
                    continue;
                }
                let function = request["function_id"].as_str().unwrap();
                let result =
                    shared
                        .lock()
                        .unwrap()
                        .rpc(function, &request["data"], &request["action"]);
                let mut reply = json!({"type":"invocationresult","invocation_id":request["invocation_id"],"function_id":function});
                match result {
                    Ok(v) => reply["result"] = v,
                    Err(e) => reply["error"] = json!({"code":"offline_denied","message":e}),
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
        let cfg = Arc::new(tokio::sync::RwLock::new(Arc::new(
            crate::config::WorkerConfig {
                session_timeout_ms: 1000,
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
        let stack = Self { deps, data, server };
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
        .expect("the recall capture observer is accepted");
        stack
    }
    async fn reset_turn(&self, session: &str, status: crate::types::turn::TurnStatus) {
        let mut r = crate::types::turn::tests::record();
        r.session_id = session.into();
        r.turn_id = format!("t-{session}");
        r.status = status;
        r.step = 0;
        r.options.system_prompt = Some("Frozen offline prompt body".into());
        r.options.system_prompt_ref = None;
        r.options.skill_context = None;
        r.options.functions = None;
        crate::state::put_turn(&self.deps.iii, &r, 1000)
            .await
            .unwrap();
    }
    async fn record(&self, session: &str) -> TurnRecord {
        crate::state::get_turn(&self.deps.iii, session, 1000)
            .await
            .unwrap()
            .unwrap()
    }
    async fn context(&self, session: &str) -> Value {
        let mut r = self.record(session).await;
        let entries = self
            .deps
            .session()
            .await
            .messages_strict(session)
            .await
            .unwrap();
        r.recall_context = resolve(&self.deps, &r, &entries).await;
        // Exercise the production dehydrator/hydrator, not raw state parsing.
        crate::state::put_turn(&self.deps.iii, &r, 1000)
            .await
            .unwrap();
        self.record(session).await.recall_context.unwrap()
    }
}
// Keep the original child/queue hook cases below unchanged. These repeated
// parallel stacks additionally assert durable equal-body isolation directly,
// before hydration can conceal an absent write behind an immutable cache hit.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_equal_frozen_body_persists_across_independent_child_and_queue_stacks() {
    for _ in 0..3 {
        let (child, queued) = tokio::join!(Stack::new(), Stack::new());
        tokio::join!(
            child.reset_turn("child", crate::types::turn::TurnStatus::Completed),
            queued.reset_turn("queued", crate::types::turn::TurnStatus::Running)
        );
        let body = "Frozen offline prompt body";
        let digest = crate::state::prompt_digest(body);
        for (stack, session) in [(&child, "child"), (&queued, "queued")] {
            {
                let data = stack.data.lock().unwrap();
                assert_eq!(data.get(crate::state::PROMPT_SCOPE, &digest)["body"], body);
                assert_eq!(
                    data.get(crate::state::TURN_SCOPE, session)["options"]["system_prompt"],
                    json!({"$ref":digest})
                );
            }
            assert_eq!(
                stack.record(session).await.options.system_prompt.as_deref(),
                Some(body)
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_invoke_all_send_destinations_keep_agent_channel_and_real_lock() {
    for target in ["self", "cross", "new"] {
        for locked in [false, true] {
            let stack = Stack::new().await;
            let caller = "calling-agent";
            stack
                .reset_turn(caller, crate::types::turn::TurnStatus::Completed)
                .await;
            let destination = match target {
                "self" => caller,
                "cross" => "other-agent",
                _ => "created-agent-session",
            };
            if target == "cross" {
                stack
                    .reset_turn(destination, crate::types::turn::TurnStatus::Completed)
                    .await;
            }
            let held = if locked {
                Some(stack.deps.locks.guard(caller).await)
            } else {
                None
            };
            let policy =
                crate::policy::CompiledPolicy::from(Some(&crate::types::turn::FunctionPolicy {
                    allow: vec![crate::functions::SEND_ID.into()],
                    deny: Vec::new(),
                    expose: Default::default(),
                }));
            let mut args = json!({"message":"Ledgerly PostgreSQL", "model":"inert-model", "options":{"thinking_level":"low","metadata":{"origin":{"kind":"direct"},"caller_holds_session_lock":true}}});
            if target != "new" {
                args["session_id"] = json!(destination);
            }
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(4),
                crate::functions::observe::invoke(
                    &stack.deps,
                    &stack.deps.engine().await,
                    &policy,
                    crate::functions::SEND_ID,
                    &args,
                    caller,
                    locked,
                    None,
                ),
            )
            .await
            .expect("send must not deadlock the caller lock");
            assert!(!result.is_error, "{target}/{locked}: {:?}", result.details);
            drop(held);
            assert_eq!(
                stack.context(destination).await["source"]["kind"],
                "invoked-agent"
            );
            assert!(
                !stack
                    .data
                    .lock()
                    .unwrap()
                    .calls
                    .iter()
                    .any(|(f, _)| f == crate::functions::SEND_ID),
                "agent send must never be reclassified by engine dispatch"
            );
            {
                let data = stack.data.lock().unwrap();
                let dispatches = crate::functions::delete_session_tree::DISPATCHES;
                let opened = data
                    .calls
                    .iter()
                    .find(|(function, payload)| {
                        function == "harness::state::compare-and-set"
                            && payload["scope"] == dispatches
                            && payload["value"]["session_id"] == caller
                            && payload["value"]["function_id"] == crate::functions::SEND_ID
                    })
                    .expect("local sends still persist the caller's deletion witness");
                assert!(
                    data.calls.iter().any(|(function, payload)| {
                        function == "harness::state::compare-and-set"
                            && payload["scope"] == dispatches
                            && payload["key"] == opened.1["key"]
                            && payload["expected"] == opened.1["value"]
                            && payload["value"].is_null()
                    }),
                    "confirmed local sends clear their exact witness by CAS"
                );
            }
            let invalid = crate::functions::observe::invoke(
                &stack.deps,
                &stack.deps.engine().await,
                &policy,
                crate::functions::SEND_ID,
                &json!({"message":42}),
                caller,
                false,
                None,
            )
            .await;
            assert!(invalid.is_error);
            let denied = crate::policy::CompiledPolicy::from(None);
            let result = crate::functions::observe::invoke(
                &stack.deps,
                &stack.deps.engine().await,
                &denied,
                crate::functions::SEND_ID,
                &args,
                caller,
                false,
                None,
            )
            .await;
            assert!(result.is_error);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_invoked_send_respects_caller_and_target_deletion_tombstones() {
    for blocked in ["caller", "target"] {
        let stack = Stack::new().await;
        for session in ["caller", "target"] {
            stack
                .reset_turn(session, crate::types::turn::TurnStatus::Completed)
                .await;
        }
        stack.data.lock().unwrap().state.insert(
            (
                crate::functions::delete_session_tree::GUARDS.into(),
                blocked.into(),
            ),
            json!("synthetic-deletion"),
        );
        let policy =
            crate::policy::CompiledPolicy::from(Some(&crate::types::turn::FunctionPolicy {
                allow: vec![crate::functions::SEND_ID.into()],
                deny: Vec::new(),
                expose: Default::default(),
            }));
        let result = crate::functions::observe::invoke(
            &stack.deps,
            &stack.deps.engine().await,
            &policy,
            crate::functions::SEND_ID,
            &json!({"session_id":"target","message":"must not arrive","model":"inert-model"}),
            "caller",
            false,
            None,
        )
        .await;
        assert!(result.is_error, "{blocked}: {:?}", result.details);
        let data = stack.data.lock().unwrap();
        assert!(
            data.get(SCOPE, "target").is_null(),
            "a refused send cannot mint recall authority"
        );
        assert!(
            !data.messages.contains_key("target"),
            "a refused send cannot append a message"
        );
        assert!(
            !data
                .calls
                .iter()
                .any(|(function, _)| function == crate::functions::SEND_ID),
            "a refused agent send cannot escape through engine dispatch"
        );
        assert!(
            data.state
                .iter()
                .filter(|((scope, _), _)| {
                    scope == crate::functions::delete_session_tree::DISPATCHES
                })
                .all(|(_, value)| value.is_null()),
            "no invocation was left running"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_authorized_direct_edit_preserves_channel_without_promoting_other_rows() {
    let stack = Stack::new().await;
    stack
        .reset_turn("direct-edit", crate::types::turn::TurnStatus::Running)
        .await;
    crate::functions::send::handle(
        &stack.deps,
        request("direct-edit", "Original direct query", None),
    )
    .await
    .unwrap();
    let row = crate::state::list_queued(&stack.deps.iii, "direct-edit", 1000)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let before = stack.record("direct-edit").await.options.system_prompt;
    let edited = crate::functions::send::edit_queued(
        &stack.deps,
        crate::functions::send::EditQueuedRequest {
            session_id: "direct-edit".into(),
            entry_id: row.entry_id.clone(),
            message: crate::functions::send::MessageInput::Text("Edited direct query".into()),
        },
    )
    .await
    .unwrap();
    assert!(edited.updated);
    let mut borrowed = row.clone();
    borrowed.session_id = "different-session".into();
    crate::state::enqueue_message(&stack.deps.iii, &borrowed, 1000)
        .await
        .unwrap();
    let cross = crate::functions::send::edit_queued(
        &stack.deps,
        crate::functions::send::EditQueuedRequest {
            session_id: "different-session".into(),
            entry_id: row.entry_id.clone(),
            message: crate::functions::send::MessageInput::Text("Cross edit".into()),
        },
    )
    .await
    .unwrap();
    assert!(cross.updated);
    stack
        .reset_turn("different-session", crate::types::turn::TurnStatus::Running)
        .await;
    crate::turn_loop::drain_queued(
        &stack.deps,
        &stack.deps.session().await,
        "different-session",
    )
    .await
    .unwrap();
    assert_eq!(
        stack.context("different-session").await["source"]["kind"],
        "unknown",
        "queued row from another session cannot borrow its receipt"
    );
    crate::turn_loop::drain_queued(&stack.deps, &stack.deps.session().await, "direct-edit")
        .await
        .unwrap();
    let context = stack.context("direct-edit").await;
    assert_eq!(context["source"]["kind"], "direct");
    assert_eq!(context["query"]["text"], "Edited direct query");
    assert_eq!(
        stack.record("direct-edit").await.options.system_prompt,
        before,
        "edit cannot rewrite the frozen prompt prefix"
    );
    let entries = stack.data.lock().unwrap().messages["direct-edit"].clone();
    stack
        .reset_turn("fork", crate::types::turn::TurnStatus::Completed)
        .await;
    stack
        .data
        .lock()
        .unwrap()
        .messages
        .insert("fork".into(), entries);
    assert_eq!(
        stack.context("fork").await["source"]["kind"],
        "unknown",
        "copied transcript origin is not authority"
    );
    crate::functions::send::handle(&stack.deps, request("direct-edit", "Next original", None))
        .await
        .unwrap();
    let mut forged = crate::state::list_queued(&stack.deps.iii, "direct-edit", 1000)
        .await
        .unwrap()
        .pop()
        .unwrap();
    forged.message = AgentMessage::user_text("Tampered old text");
    crate::state::enqueue_message(&stack.deps.iii, &forged, 1000)
        .await
        .unwrap();
    assert!(crate::functions::send::edit_queued(
        &stack.deps,
        crate::functions::send::EditQueuedRequest {
            session_id: "direct-edit".into(),
            entry_id: forged.entry_id,
            message: crate::functions::send::MessageInput::Text("Promote tamper".into())
        }
    )
    .await
    .is_err());
}

fn request(session: &str, text: &str, prior: Option<&str>) -> crate::functions::send::SendRequest {
    serde_json::from_value(json!({"session_id":session,"message":{"role":"user","content":[{"type":"text","text":text}],"timestamp":0,"origin":{"kind":"direct"}},
        "options":{"continuation_entry_id":prior,"metadata":{"origin":{"kind":"direct"},"recall_context":{"source":{"kind":"direct"}}}}})).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_delivery_real_send_invoked_and_notification_ignore_claims() {
    let stack = Stack::new().await;
    for (session, expected) in [
        ("direct", "direct"),
        ("invoked", "invoked-agent"),
        ("notice", "notification"),
    ] {
        stack
            .reset_turn(session, crate::types::turn::TurnStatus::Completed)
            .await;
        match session {
            "direct" => {
                crate::functions::send::handle(
                    &stack.deps,
                    request(session, "Ledgerly PostgreSQL", None),
                )
                .await
                .unwrap();
            }
            "invoked" => {
                crate::functions::send::handle_from_invoke(
                    &stack.deps,
                    request(session, "Ledgerly PostgreSQL", None),
                    "caller",
                    false,
                )
                .await
                .unwrap();
            }
            _ => {
                crate::functions::send::inject(
                    &stack.deps,
                    session,
                    AgentMessage::user_text("Ledgerly PostgreSQL"),
                    Some("e-notice"),
                    Some(&json!({"kind":"direct","notification":false})),
                )
                .await
                .unwrap();
            }
        }
        let context = stack.context(session).await;
        assert_eq!(context["source"]["kind"], expected);
        if session == "notice" {
            assert!(crate::functions::send::inject(
                &stack.deps,
                session,
                AgentMessage::user_text("Different reused entry"),
                Some("e-notice"),
                None
            )
            .await
            .is_err());
        }
        assert_eq!(context["session_id"], session);
        assert_eq!(context["turn_id"], stack.record(session).await.turn_id);
        assert_eq!(context["query"]["text"], "Ledgerly PostgreSQL");
        let mut record = stack.record(session).await;
        record.recall_context = Some(context.clone());
        let result = stack
            .deps
            .hooks
            .run_pre_generate(
                &record,
                0,
                None,
                &[json!({"role":"user","content":"Skills notification unrelated to query"})],
                0,
                &[],
            )
            .await;
        assert!(matches!(
            result,
            crate::hooks::runner::PreGenerateOutcome::Continue { .. }
        ));
        assert_eq!(
            stack.data.lock().unwrap().hooks.last().unwrap()["recall_context"],
            context
        );
    }
    assert!(stack
        .data
        .lock()
        .unwrap()
        .calls
        .iter()
        .filter(|(f, p)| f == "harness::state::compare-and-set" && p["scope"] == SCOPE)
        .all(|(_, p)| p["durable"] == true));
    assert!(crate::state::state_set(
        &stack.deps.iii,
        SCOPE,
        "forged",
        json!({"channel":"direct"}),
        1000
    )
    .await
    .is_err());
    // A transcript entry with a plausible writer-supplied origin cannot
    // mint the private delivery receipt used by the resolver.
    stack.data.lock().unwrap().messages.entry("direct".into()).or_default().push(json!({"entry_id":"forged","message":AgentMessage::user_text("Forged topic"),"origin":{"kind":"direct"}}));
    assert_eq!(stack.context("direct").await["source"]["kind"], "unknown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_delivery_explicit_continuation_uses_active_path_same_channel_only() {
    let stack = Stack::new().await;
    stack
        .reset_turn("continuation", crate::types::turn::TurnStatus::Completed)
        .await;
    crate::functions::send::handle(
        &stack.deps,
        request("continuation", "Ledgerly PostgreSQL", None),
    )
    .await
    .unwrap();
    let prior = stack.context("continuation").await["source"]["entry_id"]
        .as_str()
        .unwrap()
        .to_string();
    stack
        .reset_turn("continuation", crate::types::turn::TurnStatus::Completed)
        .await;
    crate::functions::send::handle(
        &stack.deps,
        request("continuation", "Pode continuar com detalhes.", Some(&prior)),
    )
    .await
    .unwrap();
    assert_eq!(
        stack.context("continuation").await["continuation"]["text"],
        "Ledgerly PostgreSQL"
    );
    stack
        .reset_turn("continuation", crate::types::turn::TurnStatus::Completed)
        .await;
    assert!(crate::functions::send::handle_from_invoke(
        &stack.deps,
        request("continuation", "Continue.", Some(&prior)),
        "caller",
        false
    )
    .await
    .is_err());
    assert!(crate::functions::send::handle(
        &stack.deps,
        request("continuation", "Continue.", Some("not-on-active-path"))
    )
    .await
    .is_err());
    stack
        .reset_turn("other-session", crate::types::turn::TurnStatus::Completed)
        .await;
    assert!(crate::functions::send::handle(
        &stack.deps,
        request("other-session", "Continue.", Some(&prior))
    )
    .await
    .is_err());
    // Leaving the active path invalidates the previously resolved reference.
    stack
        .data
        .lock()
        .unwrap()
        .messages
        .get_mut("continuation")
        .unwrap()
        .retain(|e| e["entry_id"] != prior);
    assert_eq!(
        stack.context("continuation").await["source"]["kind"],
        "unknown"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_delivery_queue_drain_recovery_retains_internal_notification_channel() {
    let stack = Stack::new().await;
    stack
        .reset_turn("queued", crate::types::turn::TurnStatus::Running)
        .await;
    let out = crate::functions::send::inject(
        &stack.deps,
        "queued",
        AgentMessage::user_text("Ledgerly PostgreSQL finished"),
        Some("e-queued"),
        Some(&json!({"kind":"direct"})),
    )
    .await
    .unwrap();
    assert!(out.queued);
    assert!(!stack.data.lock().unwrap().messages.contains_key("queued"));
    let fresh = Deps::new(
        stack.deps.iii.clone(),
        stack.deps.config.clone(),
        crate::discovery::new_cell(),
        crate::skills::new_cell(),
        stack.deps.events.clone(),
        stack.deps.hooks.clone(),
    );
    assert_eq!(
        crate::turn_loop::drain_queued(&fresh, &fresh.session().await, "queued")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        crate::turn_loop::drain_queued(&fresh, &fresh.session().await, "queued")
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        stack.context("queued").await["source"]["kind"],
        "notification"
    );
    let entries = stack
        .deps
        .session()
        .await
        .messages_strict("queued")
        .await
        .unwrap();
    let original = stack.record("queued").await;
    assert_eq!(
        resolve(&fresh, &original, &entries).await.unwrap()["source"]["kind"],
        "notification"
    );
    // Editing a queued message may alter its text but never grants authority
    // to the edited text: the immutable receipt then mismatches => unknown.
    crate::functions::send::inject(
        &stack.deps,
        "queued",
        AgentMessage::user_text("Old notification"),
        Some("e-edited"),
        None,
    )
    .await
    .unwrap();
    crate::functions::send::edit_queued(
        &stack.deps,
        crate::functions::send::EditQueuedRequest {
            session_id: "queued".into(),
            entry_id: "e-edited".into(),
            message: crate::functions::send::MessageInput::Text(
                "Ledgerly PostgreSQL edited".into(),
            ),
        },
    )
    .await
    .unwrap();
    crate::turn_loop::drain_queued(&fresh, &fresh.session().await, "queued")
        .await
        .unwrap();
    assert_eq!(stack.context("queued").await["source"]["kind"], "unknown");
    assert!(!stack
        .data
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|(f, _)| f.starts_with("router::")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_delivery_child_lineage_stays_agent_and_legacy_has_no_authority() {
    let stack = Stack::new().await;
    stack
        .reset_turn("child", crate::types::turn::TurnStatus::Completed)
        .await;
    let options = stack.record("child").await.options;
    let lineage = crate::functions::send::TurnLineage {
        depth: 1,
        parent: Some(crate::types::turn::ParentLink {
            session_id: "parent".into(),
            turn_id: "parent-turn".into(),
            function_call_id: "spawn-call".into(),
        }),
        display_parent_session_id: None,
        dispatch_only_functions: Vec::new(),
    };
    crate::functions::send::deliver(
        &stack.deps,
        stack.deps.cfg().await.as_ref(),
        "child",
        options,
        crate::functions::send::Delivery {
            channel: DeliveryChannel::InvokedAgent,
            continuation_entry_id: None,
            message: &AgentMessage::user_text("Ledgerly PostgreSQL"),
            entry_id: Some("child-entry"),
            origin: Some(&json!({"kind":"direct"})),
            lineage: &lineage,
            caller_holds_session_lock: false,
            skills_explicit: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        stack.context("child").await["source"]["kind"],
        "invoked-agent"
    );
    let record = stack.record("child").await;
    assert_eq!(record.depth, 1);
    assert_eq!(record.parent.unwrap().turn_id, "parent-turn");
    stack
        .reset_turn("legacy", crate::types::turn::TurnStatus::Completed)
        .await;
    stack.data.lock().unwrap().messages.insert("legacy".into(),vec![json!({"entry_id":"old","message":AgentMessage::user_text("Ledgerly PostgreSQL"),"origin":{"kind":"direct"}})]);
    assert_eq!(stack.context("legacy").await["source"]["kind"], "unknown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_receipt_purge_only_clears_the_selected_session() {
    let stack = Stack::new().await;
    for session in ["selected", "other"] {
        stack
            .reset_turn(session, crate::types::turn::TurnStatus::Completed)
            .await;
        crate::functions::send::inject(
            &stack.deps,
            session,
            AgentMessage::user_text("Sensitive topic"),
            Some("e1"),
            None,
        )
        .await
        .unwrap();
    }
    let before = stack.data.lock().unwrap().get(SCOPE, "other");
    assert!(!stack
        .data
        .lock()
        .unwrap()
        .get(SCOPE, "selected")
        .to_string()
        .contains("Sensitive topic"));
    purge(&stack.deps, "selected").await.unwrap();
    purge(&stack.deps, "selected").await.unwrap();
    assert!(stack.data.lock().unwrap().get(SCOPE, "selected").is_null());
    assert_eq!(stack.data.lock().unwrap().get(SCOPE, "other"), before);
    assert_eq!(stack.context("selected").await["source"]["kind"], "unknown");
    assert_eq!(
        stack.context("other").await["source"]["kind"],
        "notification"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_authority_agent_cannot_write_private_receipts_even_with_allow_all() {
    let stack = Stack::new().await;
    let policy = crate::policy::CompiledPolicy::from(Some(&crate::types::turn::FunctionPolicy {
        allow: vec!["*".into()],
        deny: Vec::new(),
        expose: Default::default(),
    }));
    let before = stack
        .data
        .lock()
        .unwrap()
        .calls
        .iter()
        .filter(|(f, _)| f != "engine::workers::register")
        .count();
    for function in [
        "harness::state::compare-and-set",
        "harness::state::get",
        "state::claim-namespace",
    ] {
        let result = crate::functions::observe::invoke(
            &stack.deps,
            &stack.deps.engine().await,
            &policy,
            function,
            &json!({"scope":SCOPE,"key":"victim","value":[{"channel":"direct"}]}),
            "attacker",
            false,
            None,
        )
        .await;
        assert!(result.is_error);
        assert_eq!(result.details["error"], "policy_denied");
    }
    assert_eq!(
        stack
            .data
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(f, _)| f != "engine::workers::register")
            .count(),
        before,
        "denied locally before any transport call"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_edit_guard_denies_agents_before_rpc_and_preserves_operator_edit() {
    let stack = Stack::new().await;
    let victim = "direct-edit-victim";
    stack
        .reset_turn(victim, crate::types::turn::TurnStatus::Running)
        .await;
    stack
        .reset_turn("editing-agent", crate::types::turn::TurnStatus::Completed)
        .await;
    crate::functions::send::handle(&stack.deps, request(victim, "Original direct query", None))
        .await
        .unwrap();
    let rows = crate::state::list_queued(&stack.deps.iii, victim, 1000)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    let before_ledger = stack.data.lock().unwrap().get(SCOPE, victim);
    assert_eq!(before_ledger.as_array().unwrap().len(), 1);
    assert_eq!(before_ledger[0]["channel"], "direct");
    let before_queue: Vec<_> = stack
        .data
        .lock()
        .unwrap()
        .state
        .iter()
        .filter(|((scope, _), _)| scope == crate::state::QUEUE_SCOPE)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let engine = stack.deps.engine().await;

    for allow in [crate::functions::EDIT_QUEUED_ID, "*"] {
        let policy =
            crate::policy::CompiledPolicy::from(Some(&crate::types::turn::FunctionPolicy {
                allow: vec![allow.into()],
                deny: Vec::new(),
                expose: Default::default(),
            }));
        assert!(policy.allows(crate::functions::EDIT_QUEUED_ID));
        for caller in ["editing-agent", victim] {
            for locked in [false, true] {
                let held = if locked {
                    Some(stack.deps.locks.guard(caller).await)
                } else {
                    None
                };
                for arguments in [
                    json!({"session_id":victim,"entry_id":row.entry_id,"message":"Agent replacement",
                        "metadata":{"origin":{"kind":"direct"}},"caller_holds_session_lock":true}),
                    json!({"session_id":victim,"entry_id":row.entry_id,"message":42}),
                    json!("invalid edit payload"),
                ] {
                    let calls_before = stack
                        .data
                        .lock()
                        .unwrap()
                        .calls
                        .iter()
                        .filter(|(function, _)| function != "engine::workers::register")
                        .count();
                    let denied = tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        crate::functions::observe::invoke(
                            &stack.deps,
                            &engine,
                            &policy,
                            crate::functions::EDIT_QUEUED_ID,
                            &arguments,
                            caller,
                            locked,
                            None,
                        ),
                    )
                    .await
                    .expect(
                        "an agent edit must be denied before waiting for a session lock or RPC",
                    );
                    assert!(denied.is_error, "{allow}/{caller}/{locked}");
                    assert_eq!(
                        denied.details["error"], "policy_denied",
                        "{allow}/{caller}/{locked}: {:?}",
                        denied.details
                    );
                    assert_eq!(
                        denied.details["function_id"],
                        crate::functions::EDIT_QUEUED_ID
                    );
                    let data = stack.data.lock().unwrap();
                    let after_queue: Vec<_> = data
                        .state
                        .iter()
                        .filter(|((scope, _), _)| scope == crate::state::QUEUE_SCOPE)
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect();
                    assert_eq!(
                        after_queue, before_queue,
                        "agent edit changed the prepared direct queue"
                    );
                    assert_eq!(
                        data.get(SCOPE, victim),
                        before_ledger,
                        "agent edit changed the private direct receipt"
                    );
                    assert_eq!(
                        data.calls
                            .iter()
                            .filter(|(function, _)| function != "engine::workers::register")
                            .count(),
                        calls_before,
                        "editor controls must be refused locally, not forwarded to any RPC"
                    );
                }
                drop(held);
            }
        }
    }

    // The operator entry point bypasses the agent boundary deliberately. The
    // same prepared direct row must still be editable, retaining its channel.
    let edited = crate::functions::send::edit_queued(
        &stack.deps,
        crate::functions::send::EditQueuedRequest {
            session_id: victim.into(),
            entry_id: row.entry_id.clone(),
            message: crate::functions::send::MessageInput::Text("Operator replacement".into()),
        },
    )
    .await
    .unwrap();
    assert!(edited.updated);
    let after_ledger = stack.data.lock().unwrap().get(SCOPE, victim);
    assert_eq!(after_ledger[0]["channel"], "direct");
    assert_ne!(
        after_ledger[0]["query"]["digest"],
        before_ledger[0]["query"]["digest"]
    );
    crate::turn_loop::drain_queued(&stack.deps, &stack.deps.session().await, victim)
        .await
        .unwrap();
    let context = stack.context(victim).await;
    assert_eq!(context["source"]["kind"], "direct");
    assert_eq!(context["source"]["entry_id"], row.entry_id);
    assert_eq!(context["query"]["text"], "Operator replacement");
}

#[path = "extraction_provenance_tests.rs"]
mod extraction_provenance_tests;
