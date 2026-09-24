//! Behavioral tests execute the production handlers through an isolated mock
//! SDK transport. No deployed engine, real session, model, or worker restart.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use harness::config::WorkerConfig;
use harness::deps::Deps;
use harness::functions::delete_session_tree::{
    self as deletion, DeleteRequest, DeletionStatus, StatusRequest,
};
use harness::types::turn::TurnRecord;
use iii_sdk::{register_worker, InitOptions};
use serde_json::{json, Value};
use tokio::sync::{oneshot, Notify, RwLock};

#[derive(Default)]
struct Store {
    state: BTreeMap<(String, String), Value>,
    sessions: BTreeMap<String, Value>,
    messages: BTreeMap<String, Vec<Value>>,
    calls: Vec<(String, Value)>,
    fail: BTreeSet<String>,
    fail_delete: Option<String>,
    delete_reply: Option<Value>,
    fail_after_queue_once: bool,
}
impl Store {
    fn state(&self, scope: &str, key: &str) -> Value {
        self.state
            .get(&(scope.into(), key.into()))
            .cloned()
            .unwrap_or(Value::Null)
    }
    fn put(&mut self, scope: &str, key: &str, value: Value) {
        self.state.insert((scope.into(), key.into()), value);
    }
    fn respond(&mut self, function: &str, data: &Value, action: &Value) -> Result<Value, String> {
        self.calls.push((function.into(), data.clone()));
        if self.fail.contains(function) {
            return Err(format!("injected failure: {function}"));
        }
        if action["type"] == "enqueue" {
            return Ok(json!({"message_receipt_id":"receipt"}));
        }
        let sid = data["session_id"].as_str().unwrap_or_default();
        let scope = data["scope"].as_str().unwrap_or_default();
        let key = data["key"].as_str().unwrap_or_default();
        match function {
            "state::set" | "harness::state::set" => {
                self.put(scope, key, data["value"].clone());
                if self.fail_after_queue_once && scope == "harness_queue" {
                    self.fail_after_queue_once = false;
                    return Err("injected acknowledgement loss after queue persistence".into());
                }
                Ok(json!({}))
            }
            "state::delete" | "harness::state::delete" => {
                self.state.remove(&(scope.into(), key.into()));
                Ok(json!({}))
            }
            "state::get" | "harness::state::get" => Ok(self.state(scope, key)),
            "state::list" | "harness::state::list" => Ok(Value::Array(
                self.state
                    .iter()
                    .filter(|((s, _), _)| s == scope)
                    .map(|(_, v)| v.clone())
                    .collect(),
            )),
            "harness::state::compare-and-set" => {
                let old = self.state(scope, key);
                let swapped = old == data["expected"];
                if swapped {
                    self.put(scope, key, data["value"].clone());
                }
                Ok(json!({"swapped":swapped,"current":old}))
            }
            "session::get" => Ok(self
                .sessions
                .get(sid)
                .map(|meta| json!({"meta":meta}))
                .unwrap_or(Value::Null)),
            "session::list" => {
                let parent = &data["metadata"]["parent_session_id"];
                Ok(
                    json!({"sessions":self.sessions.values().filter(|m| &m["metadata"]["parent_session_id"] == parent).cloned().collect::<Vec<_>>()}),
                )
            }
            "session::messages" => {
                Ok(json!({"messages":self.messages.get(sid).cloned().unwrap_or_default()}))
            }
            "session::append" => {
                if !self.sessions.contains_key(sid) {
                    return Err("append must never recreate missing parent".into());
                }
                let entry_id = data["entry_id"].clone();
                let entries = self.messages.entry(sid.into()).or_default();
                if !entries.iter().any(|entry| entry["entry_id"] == entry_id) {
                    entries.push(json!({"entry_id":entry_id,"message":data["message"],"custom":data["custom"]}));
                }
                Ok(json!({"entry_id":entry_id}))
            }
            "session::delete" => {
                if let Some(reply) = &self.delete_reply {
                    return Ok(reply.clone());
                }
                if self.fail_delete.as_deref() == Some(sid) {
                    return Err("injected deletion failure".into());
                }
                let turn = self.state("harness_turn", sid);
                assert!(
                    turn.is_null()
                        || ["completed", "cancelled", "failed"]
                            .iter()
                            .any(|s| turn["status"] == *s),
                    "deleted before terminal: {sid}"
                );
                let existed = self.sessions.remove(sid).is_some();
                self.messages.remove(sid);
                Ok(json!({"deleted":existed}))
            }
            "session::set-status" | "approval::on-session-deleted" => Ok(json!({"ok":true})),
            "approval::list-pending" => Ok(json!({"pending":[]})),
            "approval::get-settings" => Ok(json!({"source":"defaults"})),
            "router::abort" => Ok(json!({"aborted":true})),
            "context::assemble" => {
                Err("intentional boundary stop after observing LLM input".into())
            }
            "state::claim-namespace" => Ok(json!({"claimed":true})),
            "engine::unregister_trigger" => Ok(json!({"removed":true})),
            _ => Err(format!("unexpected mock RPC {function}: {data}")),
        }
    }
}

struct Stack {
    deps: Deps,
    store: Arc<Mutex<Store>>,
    changed: Arc<Notify>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Stack {
    fn drop(&mut self) {
        self.deps.iii.shutdown();
        self.server.abort();
    }
}
impl Stack {
    async fn new(parent_status: &str) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let store = Arc::new(Mutex::new(Store::default()));
        let changed = Arc::new(Notify::new());
        let (ready_tx, ready_rx) = oneshot::channel();
        let (state, notice) = (store.clone(), changed.clone());
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let _ = ready_tx.send(());
            while let Some(Ok(frame)) = socket.next().await {
                let Ok(text) = frame.to_text() else {
                    continue;
                };
                let Ok(message) = serde_json::from_str::<Value>(text) else {
                    continue;
                };
                if message["type"] != "invokefunction" {
                    continue;
                }
                let function = message["function_id"].as_str().unwrap();
                let result =
                    state
                        .lock()
                        .unwrap()
                        .respond(function, &message["data"], &message["action"]);
                notice.notify_waiters();
                if message["invocation_id"].is_null() {
                    continue;
                }
                let mut reply = json!({"type":"invocationresult","invocation_id":message["invocation_id"],"function_id":function});
                match result {
                    Ok(value) => reply["result"] = value,
                    Err(error) => reply["error"] = json!({"code":"test_error","message":error}),
                }
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
        let iii = Arc::new(register_worker(&url, InitOptions::default()));
        ready_rx.await.unwrap();
        let cfg = WorkerConfig {
            session_timeout_ms: 2_000,
            ..WorkerConfig::default()
        };
        let deps = Deps::new(
            iii.clone(),
            Arc::new(RwLock::new(Arc::new(cfg))),
            harness::discovery::new_cell(),
            harness::skills::new_cell(),
            harness::events::TurnEvents::register(&iii),
            harness::hooks::HookRegistry::register(&iii),
        );
        let stack = Self {
            deps,
            store,
            changed,
            server,
        };
        for (id, parent, status) in [
            ("parent", None, parent_status),
            ("child1", Some("parent"), "running"),
            ("child2", Some("parent"), "completed"),
            ("grandchild1", Some("child2"), "completed"),
        ] {
            stack.session(id, parent, status);
        }
        stack
    }
    /// Reset process-local coordination without resetting the durable fixture.
    /// The transport remains in-process; this is not a real engine restart.
    fn reset_runtime(&mut self) {
        let old = &self.deps;
        let fresh = Deps::new(
            old.iii.clone(),
            old.config.clone(),
            harness::discovery::new_cell(),
            harness::skills::new_cell(),
            old.events.clone(),
            old.hooks.clone(),
        );
        assert!(!Arc::ptr_eq(&old.topology, &fresh.topology));
        assert!(!Arc::ptr_eq(&old.deletion_changed, &fresh.deletion_changed));
        self.deps = fresh;
    }

    fn session(&self, id: &str, parent: Option<&str>, status: &str) {
        let mut store = self.store.lock().unwrap();
        let metadata = parent
            .map(|p| json!({"parent_session_id":p}))
            .unwrap_or(json!({}));
        store.sessions.insert(
            id.into(),
            json!({"session_id":id,"title":id,"metadata":metadata}),
        );
        store.put("harness_turn",id,json!({"session_id":id,"turn_id":format!("t_{id}"),"status":status,
            "step":0,"turn_count":0,"depth":0,"options":{"model":"fake","max_turns":16},"created_at":1,"updated_at":1}));
    }
    fn set_status(&self, id: &str, status: &str) {
        let mut store = self.store.lock().unwrap();
        let mut turn = store.state("harness_turn", id);
        turn["status"] = json!(status);
        store.put("harness_turn", id, turn);
        self.deps.deletion_changed.notify_waiters();
    }
    async fn request(&self, id: &str) -> deletion::Snapshot {
        deletion::handle(
            &self.deps,
            serde_json::from_value(json!({
                "session_id": id,
                "_caller_worker_id": "test-console-worker"
            }))
            .expect("engine-injected caller metadata must not reject deletion"),
        )
        .await
        .unwrap()
    }
    async fn run(&self, operation_id: &str) -> deletion::Snapshot {
        tokio::time::timeout(
            Duration::from_secs(5),
            deletion::run(
                &self.deps,
                serde_json::from_value(json!({
                    "operation_id": operation_id,
                    "_caller_worker_id": "test-queue-worker"
                }))
                .expect("queued deletion accepts engine-injected caller metadata"),
            ),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap()
    }
    async fn wait_for(&self, predicate: impl Fn(&Store) -> bool) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let wake = self.changed.notified();
                tokio::pin!(wake);
                wake.as_mut().enable();
                if predicate(&self.store.lock().unwrap()) {
                    break;
                }
                wake.await;
            }
        })
        .await
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deletion_accepts_engine_caller_metadata_through_command_runner_and_status() {
    let stack = Stack::new("running").await;
    let accepted = stack.request("child2").await;
    let status_request = || {
        serde_json::from_value::<StatusRequest>(json!({
            "operation_id": accepted.operation_id,
            "_caller_worker_id": "test-console-worker"
        }))
        .expect("status accepts engine-injected caller metadata")
    };
    assert_eq!(
        deletion::status(&stack.deps, status_request())
            .await
            .unwrap(),
        Some(accepted.clone())
    );
    let done = stack.run(&accepted.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed);
    assert_eq!(done.deleted_session_ids, vec!["grandchild1", "child2"]);
    assert_eq!(
        deletion::status(&stack.deps, status_request())
            .await
            .unwrap(),
        Some(done)
    );
    assert!(serde_json::from_value::<DeleteRequest>(json!({
        "_caller_worker_id": "test-console-worker"
    }))
    .is_err());
    assert!(serde_json::from_value::<StatusRequest>(json!({
        "_caller_worker_id": "test-console-worker"
    }))
    .is_err());
    let store = stack.store.lock().unwrap();
    assert!(store.sessions.contains_key("parent"));
    assert!(store.sessions.contains_key("child1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subtree_deletion_preserves_parent_sibling_and_is_idempotent() {
    let stack = Stack::new("running").await;
    let before = stack.store.lock().unwrap().state("harness_turn", "parent");
    let accepted = stack.request("child2").await;
    assert_eq!(accepted.status, DeletionStatus::Deleting);
    assert_eq!(accepted.attempt, 1);
    assert_eq!(
        stack.request("child2").await.operation_id,
        accepted.operation_id
    );
    let done = stack.run(&accepted.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    assert_eq!(done.deleted_session_ids, vec!["grandchild1", "child2"]);
    assert_eq!(stack.request("child2").await, done);
    assert_eq!(stack.run(&accepted.operation_id).await, done);
    let store = stack.store.lock().unwrap();
    assert_eq!(
        store.sessions.keys().cloned().collect::<Vec<_>>(),
        vec!["child1", "parent"]
    );
    assert_eq!(store.state("harness_turn", "parent"), before);
    let queued: Vec<_> = store
        .state
        .iter()
        .filter(|((scope, _), _)| scope == "harness_queue")
        .collect();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].1["session_id"], "parent");
    assert!(queued[0].1["message"].to_string().contains("Do not await"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_selected_parent_does_not_skip_live_grandchild_or_delete_before_terminal() {
    let stack = Stack::new("running").await;
    stack.set_status("grandchild1", "running");
    let accepted = stack.request("child2").await;
    let deps = stack.deps.clone();
    let id = accepted.operation_id.clone();
    let running = tokio::spawn(async move {
        deletion::run(&deps, StatusRequest { operation_id: id })
            .await
            .unwrap()
            .unwrap()
    });
    stack
        .wait_for(|s| s.state("harness_turn", "grandchild1")["abort"] == true)
        .await;
    assert!(!running.is_finished());
    assert!(stack.store.lock().unwrap().sessions.contains_key("child2"));
    // Models the actual turn completion event, not stopping:true.
    stack.set_status("grandchild1", "cancelled");
    let done = tokio::time::timeout(Duration::from_secs(3), running)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_parent_is_seeded_once_and_active_parent_only_queues() {
    let stack = Stack::new("completed").await;
    let accepted = stack.request("child2").await;
    assert_eq!(
        stack.run(&accepted.operation_id).await.status,
        DeletionStatus::Completed
    );
    let turn = stack.store.lock().unwrap().state("harness_turn", "parent");
    assert_eq!(turn["status"], "running");
    assert_ne!(turn["turn_id"], "t_parent");
    stack.run(&accepted.operation_id).await;
    assert_eq!(
        stack.store.lock().unwrap().state("harness_turn", "parent"),
        turn
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn absent_or_deleting_parent_is_not_recreated_or_notified() {
    for absent in [false, true] {
        let stack = Stack::new("completed").await;
        if absent {
            stack.store.lock().unwrap().sessions.remove("parent");
        }
        let accepted = stack.request("child2").await;
        if !absent {
            stack
                .store
                .lock()
                .unwrap()
                .put(deletion::GUARDS, "parent", json!("other-operation"));
        }
        // Mark already planned first for the overlap case: descendant deletion
        // completes, but does not wake the now-deleting parent.
        if !absent {
            let mut store = stack.store.lock().unwrap();
            let mut op = store.state(deletion::OPERATIONS, &accepted.operation_id);
            op["planned"] = json!(true);
            op["members"] = json!(["child2", "grandchild1"]);
            op["parent"] = json!("parent");
            store.put(deletion::OPERATIONS, &accepted.operation_id, op);
        }
        assert_eq!(
            stack.run(&accepted.operation_id).await.status,
            DeletionStatus::Completed
        );
        let store = stack.store.lock().unwrap();
        assert!(!store
            .state
            .keys()
            .any(|(scope, _)| scope == "harness_queue"));
        assert_eq!(store.sessions.contains_key("parent"), !absent);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_error_is_visible_retains_data_and_retry_keeps_identity() {
    let stack = Stack::new("running").await;
    stack.set_status("grandchild1", "running");
    {
        let mut store = stack.store.lock().unwrap();
        let mut turn = store.state("harness_turn", "grandchild1");
        turn["stream_request_id"] = json!("stream");
        store.put("harness_turn", "grandchild1", turn);
        store.fail.insert("router::abort".into());
    }
    let accepted = stack.request("child2").await;
    let failed = stack.run(&accepted.operation_id).await;
    assert_eq!(failed.status, DeletionStatus::Failed);
    assert!(failed.error.unwrap().contains("router::abort"));
    assert_eq!(stack.store.lock().unwrap().sessions.len(), 4);
    stack.store.lock().unwrap().fail.clear();
    stack.set_status("grandchild1", "cancelled");
    let retry = stack.request("child2").await;
    assert_eq!(retry.operation_id, accepted.operation_id);
    assert_eq!(retry.attempt, 2);
    assert_eq!(stack.request("child2").await.attempt, 2);
    assert_eq!(
        stack.run(&retry.operation_id).await.status,
        DeletionStatus::Completed
    );
    assert_eq!(stack.request("child2").await.attempt, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_deadline_preserves_data_and_guards_send_spawn_and_queue_recovery() {
    let stack = Stack::new("running").await;
    let accepted = stack.request("child2").await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut op = store.state(deletion::OPERATIONS, &accepted.operation_id);
        op["deadline"] = json!(0);
        store.put(deletion::OPERATIONS, &accepted.operation_id, op);
    }
    assert_eq!(
        stack.run(&accepted.operation_id).await.status,
        DeletionStatus::Failed
    );
    let send = serde_json::from_value(json!({"session_id":"grandchild1","message":"no"})).unwrap();
    assert!(harness::functions::send::handle(&stack.deps, send)
        .await
        .is_err());
    let spawn=serde_json::from_value(json!({"session_id":"new-child","parent_session_id":"grandchild1","task":"no","model":"fake"})).unwrap();
    assert!(harness::subagent::spawn_child(&stack.deps, &spawn, None)
        .await
        .is_err());
    let store = stack.store.lock().unwrap();
    assert_eq!(store.sessions.len(), 4);
    assert!(!store.calls.iter().any(|(id, _)| id == "session::delete"
        || id == "session::ensure"
        || id == "session::create"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_commands_share_identity_and_overlap_fails_without_double_delete() {
    let stack = Stack::new("completed").await;
    let (a, b) = tokio::join!(stack.request("child2"), stack.request("child2"));
    assert_eq!(a, b);
    let overlap = stack.request("grandchild1").await;
    assert_eq!(overlap.status, DeletionStatus::Failed);
    assert_eq!(
        stack.run(&a.operation_id).await.status,
        DeletionStatus::Completed
    );
    let store = stack.store.lock().unwrap();
    assert_eq!(
        store
            .calls
            .iter()
            .filter(|(f, _)| f == "session::delete")
            .count(),
        2
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_flight_tool_lock_must_exit_before_deletion_and_late_descendants_are_discovered() {
    let stack = Stack::new("completed").await;
    // Models spawn metadata committed just before the admission lock is won.
    let topology = stack.deps.topology.lock().await;
    let request = deletion::handle(
        &stack.deps,
        DeleteRequest {
            session_id: "child2".into(),
        },
    );
    tokio::pin!(request);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut request)
            .await
            .is_err()
    );
    stack.session("late-child", Some("child2"), "completed");
    drop(topology);
    let accepted = request.await.unwrap();
    let held = stack.deps.turn_activity.guard("grandchild1").await;
    let deps = stack.deps.clone();
    let id = accepted.operation_id.clone();
    let job = tokio::spawn(async move {
        deletion::run(&deps, StatusRequest { operation_id: id })
            .await
            .unwrap()
            .unwrap()
    });
    stack
        .wait_for(|s| s.state(deletion::GUARDS, "late-child").is_string())
        .await;
    assert!(!job.is_finished());
    assert!(!stack
        .store
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|(f, _)| f == "session::delete"));
    drop(held);
    let done = tokio::time::timeout(Duration::from_secs(3), job)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    assert!(done.deleted_session_ids.contains(&"late-child".into()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notification_reaches_history_and_model_context_not_only_ui() {
    let stack = Stack::new("completed").await;
    let accepted = stack.request("child2").await;
    assert_eq!(
        stack.run(&accepted.operation_id).await.status,
        DeletionStatus::Completed
    );
    let turn = stack.store.lock().unwrap().state("harness_turn", "parent");
    let payload = serde_json::from_value(
        json!({"session_id":"parent", "turn_id":turn["turn_id"], "step":0, "depth":0}),
    )
    .unwrap();
    // Stop at the context boundary: no provider call is made. Production
    // run_step performs the queue drain, history read and model assembly.
    let _ = harness::turn_loop::run_step(&stack.deps, payload).await;
    let store = stack.store.lock().unwrap();
    let entries = store.messages.get("parent").unwrap();
    assert_eq!(
        entries
            .iter()
            .filter(|e| e["entry_id"]
                .as_str()
                .is_some_and(|id| id.ends_with("_parent")))
            .count(),
        1
    );
    let context = store
        .calls
        .iter()
        .find(|(f, _)| f == "context::assemble")
        .expect("notification must reach model context");
    assert!(context.1["messages"].to_string().contains("Do not await"));
    assert!(context.1["messages"].to_string().contains("grandchild1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn partial_delete_retry_keeps_plan_and_notifies_only_once() {
    let stack = Stack::new("running").await;
    stack.store.lock().unwrap().fail_delete = Some("child2".into());
    let accepted = stack.request("child2").await;
    let failed = stack.run(&accepted.operation_id).await;
    assert_eq!(failed.status, DeletionStatus::Failed);
    assert_eq!(failed.deleted_session_ids, vec!["grandchild1"]);
    assert!(stack.store.lock().unwrap().sessions.contains_key("child2"));
    stack.store.lock().unwrap().fail_delete = None;
    let retry = stack.request("child2").await;
    assert_eq!(retry.operation_id, accepted.operation_id);
    assert_eq!(
        stack.run(&retry.operation_id).await.status,
        DeletionStatus::Completed
    );
    let store = stack.store.lock().unwrap();
    assert_eq!(
        store
            .state
            .keys()
            .filter(|(scope, _)| scope == "harness_queue")
            .count(),
        1
    );
    assert_eq!(
        store
            .calls
            .iter()
            .filter(|(f, p)| f == "session::delete" && p["session_id"] == "grandchild1")
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_reenqueues_persisted_operation_after_lost_enqueue_ack() {
    let stack = Stack::new("completed").await;
    let accepted = stack.request("child2").await;
    stack.store.lock().unwrap().calls.clear();
    deletion::recover(&stack.deps).await.unwrap();
    assert!(stack
        .store
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|(f, p)| f == deletion::RUN_ID && p["operation_id"] == accepted.operation_id));
    assert_eq!(
        stack.run(&accepted.operation_id).await.status,
        DeletionStatus::Completed
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_parent_notification_ack_reuses_deterministic_queue_identity() {
    let stack = Stack::new("running").await;
    let accepted = stack.request("child2").await;
    assert_eq!(
        stack.run(&accepted.operation_id).await.status,
        DeletionStatus::Completed
    );
    {
        let mut store = stack.store.lock().unwrap();
        let mut op = store.state(deletion::OPERATIONS, &accepted.operation_id);
        op["snapshot"]["status"] = json!("failed");
        op["notified"] = json!(false);
        store.put(deletion::OPERATIONS, &accepted.operation_id, op);
    }
    let retry = stack.request("child2").await;
    assert_eq!(
        stack.run(&retry.operation_id).await.status,
        DeletionStatus::Completed
    );
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .state
            .keys()
            .filter(|(scope, _)| scope == "harness_queue")
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn durable_unknown_tool_witness_expires_failed_without_erasing_sessions() {
    let stack = Stack::new("completed").await;
    let accepted = stack.request("child2").await;
    {
        let mut store = stack.store.lock().unwrap();
        store.put(
            deletion::DISPATCHES,
            "unknown-call",
            json!({"session_id":"grandchild1","function_id":"slow::work"}),
        );
        let mut op = store.state(deletion::OPERATIONS, &accepted.operation_id);
        op["deadline"] = json!(harness::types::message::AgentMessage::now_ms() + 100);
        store.put(deletion::OPERATIONS, &accepted.operation_id, op);
    }
    let failed = stack.run(&accepted.operation_id).await;
    assert_eq!(failed.status, DeletionStatus::Failed);
    assert!(failed.error.unwrap().contains("deadline"));
    assert_eq!(stack.store.lock().unwrap().sessions.len(), 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_cascades_durable_children_even_when_selected_turn_is_terminal() {
    let stack = Stack::new("completed").await;
    stack.set_status("grandchild1", "running");
    let response = harness::functions::stop::handle(
        &stack.deps,
        harness::functions::stop::StopRequest {
            session_id: "child2".into(),
            turn_id: None,
        },
    )
    .await
    .unwrap();
    assert!(response.stopping);
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .state("harness_turn", "grandchild1")["abort"],
        true
    );
    assert_ne!(
        stack.store.lock().unwrap().state("harness_turn", "child1")["abort"],
        true
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tombstoned_wake_is_dropped_and_queued_turn_cannot_recreate_deleted_session() {
    let stack = Stack::new("completed").await;
    let accepted = stack.request("child2").await;
    let wake = harness::functions::send::inject(
        &stack.deps,
        "child2",
        harness::types::message::AgentMessage::user_text("late wake"),
        Some("late"),
        None,
    )
    .await;
    assert!(wake.is_err());
    assert_eq!(
        stack.run(&accepted.operation_id).await.status,
        DeletionStatus::Completed
    );
    let payload = serde_json::from_value(
        json!({"session_id":"child2","turn_id":"t_child2","step":0,"depth":0}),
    )
    .unwrap();
    let response = harness::turn_loop::run_step(&stack.deps, payload)
        .await
        .unwrap();
    assert!(response.skipped);
    assert!(!stack.store.lock().unwrap().sessions.contains_key("child2"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn child_first_rejects_ancestor_before_tombstone_and_keeps_parent_notifiable() {
    let stack = Stack::new("running").await;
    stack.set_status("grandchild1", "running");
    let child = stack.request("child2").await;
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .state(deletion::OPERATIONS, &child.operation_id)["planned"],
        false
    );
    let parent = stack.request("parent").await;
    assert_eq!(parent.status, DeletionStatus::Failed);
    assert!(parent.error.as_deref().unwrap().contains("overlapping"));
    for id in ["parent", "child1"] {
        assert!(stack
            .store
            .lock()
            .unwrap()
            .state(deletion::GUARDS, id)
            .is_null());
        let work = harness::functions::send::inject(
            &stack.deps,
            id,
            harness::types::message::AgentMessage::user_text("surviving work"),
            Some(&format!("work-{id}")),
            None,
        )
        .await;
        assert!(
            work.is_ok(),
            "{id}: {}",
            work.err().map(|e| e.to_string()).unwrap_or_default()
        );
    }
    let deps = stack.deps.clone();
    let operation_id = child.operation_id.clone();
    let job = tokio::spawn(async move {
        deletion::run(&deps, StatusRequest { operation_id })
            .await
            .unwrap()
            .unwrap()
    });
    stack
        .wait_for(|s| s.state("harness_turn", "grandchild1")["abort"] == true)
        .await;
    // A repeated command remains prompt while the runner owns the operation lock.
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), stack.request("child2"))
            .await
            .unwrap()
            .attempt,
        1
    );
    assert!(!job.is_finished());
    stack.set_status("grandchild1", "cancelled");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), job)
            .await
            .unwrap()
            .unwrap()
            .status,
        DeletionStatus::Completed
    );
    let store = stack.store.lock().unwrap();
    let notices: Vec<_> = store
        .state
        .iter()
        .filter(|((scope, _), row)| {
            scope == "harness_queue" && row["origin"]["deletion_operation_id"] == child.operation_id
        })
        .collect();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].1["session_id"], "parent");
    assert!(store.sessions.contains_key("parent"));
    assert!(store.sessions.contains_key("child1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_repairs_legacy_unplanned_ancestor_guard_without_losing_child_notice() {
    let stack = Stack::new("running").await;
    let child = stack.request("child2").await;
    let parent = stack.request("parent").await;
    // Reproduce the previous version's partial reservation left by rejection.
    stack
        .store
        .lock()
        .unwrap()
        .put(deletion::GUARDS, "parent", json!(parent.operation_id));
    assert_eq!(
        stack.run(&child.operation_id).await.status,
        DeletionStatus::Completed
    );
    let store = stack.store.lock().unwrap();
    assert!(store.state(deletion::GUARDS, "parent").is_null());
    assert!(store
        .state
        .iter()
        .any(|((scope, _), row)| scope == "harness_queue"
            && row["origin"]["deletion_operation_id"] == child.operation_id));
    assert_eq!(
        store.state(deletion::OPERATIONS, &parent.operation_id)["snapshot"]["status"],
        "failed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn partial_guard_recovery_rejects_conflict_without_overwriting_foreign_owner() {
    let stack = Stack::new("completed").await;
    let child = stack.request("child2").await;
    let parent = stack.request("parent").await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut old = store.state(deletion::OPERATIONS, &parent.operation_id);
        old["snapshot"]["status"] = json!("deleting");
        store.put(deletion::OPERATIONS, &parent.operation_id, old);
        store.put(deletion::GUARDS, "parent", json!(parent.operation_id));
    }
    assert_eq!(
        stack.run(&parent.operation_id).await.status,
        DeletionStatus::Failed
    );
    assert!(stack
        .store
        .lock()
        .unwrap()
        .state(deletion::GUARDS, "parent")
        .is_null());
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .state(deletion::GUARDS, "child2"),
        child.operation_id
    );
    assert_eq!(
        stack.run(&child.operation_id).await.status,
        DeletionStatus::Completed
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_retry_increments_once_and_late_terminal_event_keeps_old_attempt() {
    use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
    let stack = Stack::new("completed").await;
    stack
        .deps
        .deletion_events
        .register_trigger(TriggerConfig {
            id: "review-subscription".into(),
            function_id: "test::deletion-event".into(),
            config: json!({"session_id":"child2"}),
            metadata: None,
            namespace: None,
        })
        .await
        .unwrap();
    let accepted = stack.request("child2").await;
    stack.store.lock().unwrap().fail_delete = Some("grandchild1".into());
    let old = stack.run(&accepted.operation_id).await;
    assert_eq!(old.attempt, 1);
    stack.store.lock().unwrap().fail_delete = None;
    let (a, b) = tokio::join!(stack.request("child2"), stack.request("child2"));
    assert_eq!(a, b);
    assert_eq!(a.attempt, 2);
    deletion::recover(&stack.deps).await.unwrap();
    assert_eq!(stack.request("child2").await.attempt, 2);
    // Deliberately publish the previous terminal snapshot AFTER retry acceptance.
    stack.deps.deletion_events.emit(&old).await.unwrap();
    stack
        .wait_for(|s| {
            s.calls
                .iter()
                .filter(|(f, _)| f == "test::deletion-event")
                .count()
                >= 2
        })
        .await;
    let current = deletion::status(
        &stack.deps,
        StatusRequest {
            operation_id: a.operation_id.clone(),
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(current.status, DeletionStatus::Deleting);
    assert_eq!(current.attempt, 2);
    let emitted = stack
        .store
        .lock()
        .unwrap()
        .calls
        .iter()
        .filter(|(f, _)| f == "test::deletion-event")
        .map(|(_, p)| p.clone())
        .collect::<Vec<_>>();
    assert!(emitted
        .iter()
        .all(|p| p["attempt"] == 1 && p["status"] == "failed"));
    assert_eq!(stack.run(&a.operation_id).await.attempt, 2);
    assert_eq!(stack.request("child2").await.attempt, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_waits_for_old_runner_before_writing_new_attempt() {
    let stack = Stack::new("completed").await;
    let op = stack.request("child2").await;
    let held = stack.deps.deletion_commands.guard(&op.operation_id).await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut value = store.state(deletion::OPERATIONS, &op.operation_id);
        value["snapshot"]["status"] = json!("failed");
        store.put(deletion::OPERATIONS, &op.operation_id, value);
    }
    let retry = stack.request("child2");
    tokio::pin!(retry);
    assert!(tokio::time::timeout(Duration::from_millis(20), &mut retry)
        .await
        .is_err());
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .state(deletion::OPERATIONS, &op.operation_id)["snapshot"]["attempt"],
        1
    );
    drop(held);
    assert_eq!(retry.await.attempt, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_or_contradictory_delete_ack_does_not_report_deleted() {
    for reply in [
        json!({}),
        json!({"deleted":"true"}),
        json!({"deleted":false}),
        json!({"deleted":true}),
    ] {
        let stack = Stack::new("completed").await;
        let op = stack.request("child2").await;
        stack.store.lock().unwrap().delete_reply = Some(reply);
        let failed = stack.run(&op.operation_id).await;
        assert_eq!(failed.status, DeletionStatus::Failed);
        assert!(failed.deleted_session_ids.is_empty());
        assert!(stack
            .store
            .lock()
            .unwrap()
            .sessions
            .contains_key("grandchild1"));
        assert!(!stack
            .store
            .lock()
            .unwrap()
            .state("harness_turn", "grandchild1")
            .is_null());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_session_after_lost_delete_ack_is_an_idempotent_success() {
    let stack = Stack::new("completed").await;
    let op = stack.request("child2").await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut value = store.state(deletion::OPERATIONS, &op.operation_id);
        value["planned"] = json!(true);
        value["members"] = json!(["child2", "grandchild1"]);
        value["parent"] = json!("parent");
        store.put(deletion::OPERATIONS, &op.operation_id, value);
        store.sessions.remove("grandchild1");
    }
    let done = stack.run(&op.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed);
    assert!(done.deleted_session_ids.contains(&"grandchild1".into()));
}

#[test]
fn fixtures_are_real_turn_records() {
    let value = json!({"session_id":"s","turn_id":"t","status":"completed","step":0,"turn_count":0,"depth":0,
        "options":{"model":"fake","max_turns":16},"created_at":1,"updated_at":1});
    assert!(serde_json::from_value::<TurnRecord>(value).is_ok());
}

#[test]
fn snapshot_attempt_contract_accepts_one_and_two_but_rejects_zero_and_missing() {
    for attempt in [1, 2] {
        let value = json!({
            "operation_id": "op",
            "attempt": attempt,
            "session_id": "s",
            "status": "deleting",
            "deleted_session_ids": []
        });
        assert!(serde_json::from_value::<deletion::Snapshot>(value).is_ok());
    }
    let zero = json!({
        "operation_id": "op",
        "attempt": 0,
        "session_id": "s",
        "status": "deleting",
        "deleted_session_ids": []
    });
    assert!(serde_json::from_value::<deletion::Snapshot>(zero).is_err());
    let missing = json!({
        "operation_id": "op",
        "session_id": "s",
        "status": "deleting",
        "deleted_session_ids": []
    });
    assert!(serde_json::from_value::<deletion::Snapshot>(missing).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn corrupted_zero_attempt_status_and_run_fail_closed_without_side_effects() {
    let stack = Stack::new("completed").await;
    let accepted = stack.request("child2").await;
    let mut corrupt = stack
        .store
        .lock()
        .unwrap()
        .state(deletion::OPERATIONS, &accepted.operation_id);
    corrupt["snapshot"]["attempt"] = json!(0);
    stack
        .store
        .lock()
        .unwrap()
        .put(deletion::OPERATIONS, &accepted.operation_id, corrupt);
    let before = stack
        .store
        .lock()
        .unwrap()
        .state(deletion::OPERATIONS, &accepted.operation_id);
    let status = deletion::status(
        &stack.deps,
        StatusRequest {
            operation_id: accepted.operation_id.clone(),
        },
    )
    .await;
    assert!(status.is_err());
    let run = deletion::run(
        &stack.deps,
        StatusRequest {
            operation_id: accepted.operation_id.clone(),
        },
    )
    .await;
    let error = run
        .expect_err("corrupt storage must not enter successful run")
        .to_string();
    assert!(
        error.contains("attempt") && error.contains("at least 1"),
        "{error}"
    );
    let store = stack.store.lock().unwrap();
    assert_eq!(
        store.state(deletion::OPERATIONS, &accepted.operation_id),
        before
    );
    assert_eq!(store.sessions.len(), 4);
    assert!(!store
        .calls
        .iter()
        .any(|(function, _)| function == "session::delete"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zero_attempt_command_does_not_normalize_or_retry_and_max_attempt_overflows() {
    let stack = Stack::new("completed").await;
    let accepted = stack.request("child2").await;
    let mut corrupt = stack
        .store
        .lock()
        .unwrap()
        .state(deletion::OPERATIONS, &accepted.operation_id);
    corrupt["snapshot"]["attempt"] = json!(0);
    corrupt["snapshot"]["status"] = json!("failed");
    stack
        .store
        .lock()
        .unwrap()
        .put(deletion::OPERATIONS, &accepted.operation_id, corrupt);
    assert!(deletion::handle(
        &stack.deps,
        DeleteRequest {
            session_id: "child2".into()
        }
    )
    .await
    .is_err());
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .state(deletion::OPERATIONS, &accepted.operation_id)["snapshot"]["attempt"],
        0
    );

    let mut exhausted = stack
        .store
        .lock()
        .unwrap()
        .state(deletion::OPERATIONS, &accepted.operation_id);
    exhausted["snapshot"]["attempt"] = json!(u32::MAX);
    stack
        .store
        .lock()
        .unwrap()
        .put(deletion::OPERATIONS, &accepted.operation_id, exhausted);
    assert!(deletion::handle(
        &stack.deps,
        DeleteRequest {
            session_id: "child2".into()
        }
    )
    .await
    .is_err());
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .state(deletion::OPERATIONS, &accepted.operation_id)["snapshot"]["attempt"],
        u32::MAX
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_resumes_checkpointed_partial_plan_and_preserves_parent_and_sibling() {
    let mut stack = Stack::new("running").await;
    let accepted = stack.request("child2").await;
    stack.store.lock().unwrap().fail_delete = Some("child2".into());
    let interrupted = stack.run(&accepted.operation_id).await;
    assert_eq!(interrupted.status, DeletionStatus::Failed);
    assert_eq!(interrupted.deleted_session_ids, vec!["grandchild1"]);
    {
        let mut store = stack.store.lock().unwrap();
        // The successful grandchild checkpoint was persisted before the next
        // delete failed. Model a crash before the failure snapshot was saved.
        let mut op = store.state(deletion::OPERATIONS, &accepted.operation_id);
        assert_eq!(op["notified"], true);
        op["snapshot"]["status"] = json!("deleting");
        op["snapshot"].as_object_mut().unwrap().remove("error");
        store.put(deletion::OPERATIONS, &accepted.operation_id, op);
        store.fail_delete = None;
        store.calls.clear();
    }
    stack.reset_runtime();
    deletion::recover(&stack.deps).await.unwrap();
    assert!(stack
        .store
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|(f, p)| f == deletion::RUN_ID && p["operation_id"] == accepted.operation_id));
    let done = stack.run(&accepted.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed);
    assert_eq!(done.attempt, accepted.attempt);
    assert_eq!(done.deleted_session_ids, vec!["grandchild1", "child2"]);
    let store = stack.store.lock().unwrap();
    assert!(store.sessions.contains_key("parent"));
    assert!(store.sessions.contains_key("child1"));
    assert!(!store.sessions.contains_key("child2"));
    assert!(!store.sessions.contains_key("grandchild1"));
    assert_eq!(store.state("harness_turn", "parent")["status"], "running");
    assert_eq!(store.state("harness_turn", "child1")["status"], "running");
    let deletes: Vec<_> = store
        .calls
        .iter()
        .filter(|(f, _)| f == "session::delete")
        .map(|(_, p)| p["session_id"].as_str().unwrap())
        .collect();
    assert_eq!(deletes, vec!["child2"]);
    assert_eq!(
        store
            .state
            .iter()
            .filter(|((scope, _), _)| scope == "harness_queue")
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persisted_parent_notification_retries_once_after_ack_loss() {
    let stack = Stack::new("completed").await;
    let accepted = stack.request("child2").await;
    stack.store.lock().unwrap().fail_after_queue_once = true;
    let failed = stack.run(&accepted.operation_id).await;
    assert_eq!(failed.status, DeletionStatus::Failed);
    let queue_id = format!("e_{}_parent", accepted.operation_id);
    {
        let store = stack.store.lock().unwrap();
        let rows: Vec<_> = store
            .state
            .iter()
            .filter(|((scope, _), row)| scope == "harness_queue" && row["entry_id"] == queue_id)
            .collect();
        assert_eq!(
            rows.len(),
            1,
            "notification write survived lost acknowledgement"
        );
    }
    let retry = stack.request("child2").await;
    assert_eq!(retry.attempt, 2);
    assert_eq!(
        stack.run(&retry.operation_id).await.status,
        DeletionStatus::Completed
    );
    {
        let store = stack.store.lock().unwrap();
        let rows: Vec<_> = store
            .state
            .iter()
            .filter(|((scope, _), row)| scope == "harness_queue" && row["entry_id"] == queue_id)
            .collect();
        assert_eq!(
            rows.len(),
            1,
            "retry preserves one durable notification row"
        );
    }
    let turn = stack.store.lock().unwrap().state("harness_turn", "parent");
    let payload = serde_json::from_value(
        json!({"session_id":"parent", "turn_id":turn["turn_id"], "step":0, "depth":0}),
    )
    .unwrap();
    // Run the real drain/append/context path, stopping before a provider call.
    let _ = harness::turn_loop::run_step(&stack.deps, payload).await;
    let repeated = stack.request("child2").await;
    assert_eq!(repeated.status, DeletionStatus::Completed);
    assert_eq!(repeated.attempt, 2);
    let store = stack.store.lock().unwrap();
    let entries = store.messages.get("parent").unwrap();
    assert_eq!(
        entries.iter().filter(|e| e["entry_id"] == queue_id).count(),
        1
    );
    assert_eq!(
        store
            .calls
            .iter()
            .filter(|(f, p)| f == "session::append" && p["entry_id"] == queue_id)
            .count(),
        1
    );
    assert!(!store
        .state
        .iter()
        .any(|((scope, _), row)| scope == "harness_queue" && row["entry_id"] == queue_id));
    let context = store
        .calls
        .iter()
        .find(|(f, _)| f == "context::assemble")
        .expect("the persisted notice must reach model context after retry");
    let notices = context.1["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message.to_string().contains(&accepted.operation_id))
        .count();
    assert_eq!(notices, 1);
    assert!(context.1["messages"].to_string().contains("Do not await"));
    assert!(store.sessions.contains_key("parent"));
    assert!(store.sessions.contains_key("child1"));
}
