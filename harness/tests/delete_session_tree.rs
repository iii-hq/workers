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
    private_scopes: BTreeSet<String>,
    private_accessors_missing: bool,
    mutate_dispatch_on_read: Option<Value>,
    entry_snapshots: BTreeMap<String, Vec<(String, Value)>>,
    entry_page_overrides: BTreeMap<usize, Value>,
    fail_entry_page: Option<usize>,
    hold_later_entry_reply: Option<Arc<Notify>>,
    hold_dispatch_reply: Option<Arc<Notify>>,
    hold_target_reply: Option<(String, Arc<Notify>)>,
    fail_dispatch_clear: bool,
    fail_state_read: Option<(String, String)>,
    fail_root_release: bool,
    fail: BTreeSet<String>,
    fail_delete: Option<String>,
    delete_reply: Option<Value>,
    fail_after_delete_once: bool,
    fail_read_after_delete: bool,
    fail_session_read: Option<String>,
    fail_after_queue_once: bool,
    fail_legacy_migration_save_once: bool,
    fail_descendant_claim_once: bool,
    /// Reply to these functions with a remote error carrying this code.
    codes: BTreeMap<String, String>,
    /// Fail the deletion-runner enqueue for these operation ids.
    fail_enqueue: BTreeSet<String>,
    /// The harness's process-wide topology lock; `session::delete` records
    /// whether it was still held while the delete RPC was in flight.
    topology_probe: Option<Arc<tokio::sync::Mutex<()>>>,
    topology_held_on_delete: Vec<bool>,
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
        if self.fail.contains(function) || self.codes.contains_key(function) {
            return Err(format!("injected failure: {function}"));
        }
        if action["type"] == "enqueue" {
            if data["operation_id"]
                .as_str()
                .is_some_and(|id| self.fail_enqueue.contains(id))
            {
                return Err("injected enqueue failure".into());
            }
            return Ok(json!({"message_receipt_id":"receipt"}));
        }
        let sid = data["session_id"].as_str().unwrap_or_default();
        let scope = data["scope"].as_str().unwrap_or_default();
        let key = data["key"].as_str().unwrap_or_default();
        if self.fail_dispatch_clear
            && scope == deletion::DISPATCHES
            && function == "harness::state::compare-and-set"
            && data["value"].is_null()
        {
            return Err("injected dispatch withdrawal failure".into());
        }
        if function.starts_with("state::") && self.private_scopes.contains(scope) {
            return Err(format!(
                "RESERVED_SCOPE: `{scope}` is private worker bookkeeping"
            ));
        }
        if function.starts_with("harness::state::") {
            if self.private_accessors_missing {
                return Err("function_not_found: private accessor not registered".into());
            }
            if !self.private_scopes.contains(scope) {
                return Err(format!(
                    "INVALID_SCOPE: `{scope}` is not private state of namespace `harness`"
                ));
            }
        }
        if function == "session::get" && self.fail_session_read.as_deref() == Some(sid) {
            return Err("injected existence read failure".into());
        }
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
            "state::get" | "harness::state::get" => {
                if self.fail_state_read.as_ref() == Some(&(scope.into(), key.into())) {
                    return Err("injected diagnostic read failure".into());
                }
                let value = self.state(scope, key);
                if scope == deletion::DISPATCHES && !value.is_null() {
                    if let Some(next) = self.mutate_dispatch_on_read.take() {
                        self.put(scope, key, next);
                    }
                }
                Ok(value)
            }
            "state::list_entries" | "harness::state::list_entries" => {
                let offset = if data["cursor"].is_null() {
                    0
                } else {
                    data["cursor"]
                        .as_str()
                        .unwrap()
                        .split(':')
                        .next_back()
                        .unwrap()
                        .parse::<usize>()
                        .unwrap()
                };
                if self.fail_entry_page == Some(offset) {
                    return Err("ROW_TOO_LARGE: injected later-page failure".into());
                }
                if let Some(page) = self.entry_page_overrides.get(&offset) {
                    return Ok(page.clone());
                }
                if offset == 0 {
                    let entries: Vec<_> = self
                        .state
                        .iter()
                        .filter(|((s, _), v)| {
                            s == scope
                                && !(function == "harness::state::list_entries"
                                    && data["non_null_only"] == true
                                    && v.is_null())
                        })
                        .map(|((_, key), value)| (key.clone(), value.clone()))
                        .collect();
                    if entries.len() > 100_000 {
                        return Err("SNAPSHOT_CAPACITY: snapshot row cap".into());
                    }
                    self.entry_snapshots.insert(scope.to_owned(), entries);
                }
                let snapshot = &self.entry_snapshots[scope];
                let total = snapshot.len();
                let end = (offset + data["limit"].as_u64().unwrap_or(100) as usize).min(total);
                let entries = snapshot[offset..end].to_vec();
                if scope == deletion::DISPATCHES {
                    if let Some(next) = self.mutate_dispatch_on_read.take() {
                        if let Some((key, _)) = entries.iter().find(|(_, value)| !value.is_null()) {
                            self.put(scope, key, next);
                        }
                    }
                }
                Ok(
                    json!({"entries":entries,"offset":offset,"total":total,"done":end==total,
                    "next_cursor": if end==total { Value::Null } else { json!(format!("page:{end}")) }}),
                )
            }
            "state::list_keys" | "harness::state::list_keys" => Ok(json!({"keys": self
                .state
                .keys()
                .filter(|(s, _)| s == scope)
                .map(|(_, k)| k)
                .collect::<Vec<_>>()})),
            "state::list" | "harness::state::list" => Ok(Value::Array(
                self.state
                    .iter()
                    .filter(|((s, _), _)| s == scope)
                    .map(|(_, v)| v.clone())
                    .collect(),
            )),
            "harness::state::compare-and-set" => {
                if self.fail_root_release
                    && scope == deletion::GUARDS
                    && key == "parent"
                    && data["value"].is_null()
                {
                    return Err("injected root release failure".into());
                }
                let old = self.state(scope, key);
                let swapped = old == data["expected"];
                if swapped {
                    self.put(scope, key, data["value"].clone());
                    if self.fail_legacy_migration_save_once
                        && scope == deletion::OPERATIONS
                        && data["value"].get("cleanup_started").is_some()
                    {
                        self.fail_legacy_migration_save_once = false;
                        return Err(
                            "injected lost acknowledgement after legacy intent persistence".into(),
                        );
                    }
                    if self.fail_descendant_claim_once
                        && scope == deletion::GUARDS
                        && key == "grandchild1"
                    {
                        self.fail_descendant_claim_once = false;
                        return Err(
                            "injected lost acknowledgement after descendant reservation".into()
                        );
                    }
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
                let held = self
                    .topology_probe
                    .as_ref()
                    .map(|topology| topology.try_lock().is_err());
                if let Some(held) = held {
                    self.topology_held_on_delete.push(held);
                }
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
                if self.fail_read_after_delete {
                    self.fail_session_read = Some(sid.into());
                }
                if self.fail_after_delete_once {
                    self.fail_after_delete_once = false;
                    return Err("injected acknowledgement loss after session deletion".into());
                }
                Ok(json!({"deleted":existed}))
            }
            "session::set-status" | "approval::on-session-deleted" => Ok(json!({"ok":true})),
            "approval::list-pending" => Ok(json!({"pending":[]})),
            "approval::get-settings" => Ok(json!({"source":"defaults"})),
            "router::abort" => Ok(json!({"aborted":true})),
            "ext::approved-real" => Ok(json!({"ok":true})),
            "context::assemble" => {
                Err("intentional boundary stop after observing LLM input".into())
            }
            "state::claim-namespace" => {
                if data["functions_prefix"] != "harness" {
                    return Err("FORBIDDEN: fixture caller may only claim harness".into());
                }
                self.private_scopes.extend(
                    data["scopes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|scope| scope.as_str().unwrap().to_owned()),
                );
                self.private_accessors_missing = false;
                Ok(json!({"claimed":true}))
            }
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
        let store = Arc::new(Mutex::new(Store {
            // Model an already claimed namespace, as at ordinary Harness boot.
            private_scopes: [
                deletion::OPERATIONS,
                deletion::GUARDS,
                deletion::DISPATCHES,
                harness::state::BINDING_SCOPE,
                harness::state::BINDING_OWNER_SCOPE,
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            ..Store::default()
        }));
        let changed = Arc::new(Notify::new());
        let (ready_tx, ready_rx) = oneshot::channel();
        let (state, notice) = (store.clone(), changed.clone());
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let (mut writer, mut reader) = socket.split();
            let (replies, mut outgoing) = tokio::sync::mpsc::unbounded_channel();
            let sender = tokio::spawn(async move {
                while let Some(reply) = outgoing.recv().await {
                    if writer.send(reply).await.is_err() {
                        break;
                    }
                }
            });
            let _ = ready_tx.send(());
            while let Some(Ok(frame)) = reader.next().await {
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
                let (result, code, gate) = {
                    let mut store = state.lock().unwrap();
                    let code = store
                        .codes
                        .get(function)
                        .cloned()
                        .unwrap_or_else(|| "test_error".into());
                    let gate = if function == "harness::state::list_entries"
                        && !message["data"]["cursor"].is_null()
                    {
                        store.hold_later_entry_reply.take()
                    } else if function == "harness::state::compare-and-set"
                        && message["data"]["scope"] == deletion::DISPATCHES
                        && !message["data"]["value"].is_null()
                    {
                        store.hold_dispatch_reply.take()
                    } else if store
                        .hold_target_reply
                        .as_ref()
                        .is_some_and(|(target, _)| target == function)
                    {
                        store.hold_target_reply.take().map(|(_, gate)| gate)
                    } else {
                        None
                    };
                    (
                        store.respond(function, &message["data"], &message["action"]),
                        code,
                        gate,
                    )
                };
                notice.notify_waiters();
                if message["invocation_id"].is_null() {
                    continue;
                }
                let mut reply = json!({"type":"invocationresult","invocation_id":message["invocation_id"],"function_id":function});
                match result {
                    Ok(value) => reply["result"] = value,
                    Err(error) => reply["error"] = json!({"code":code,"message":error}),
                }
                let frame = tokio_tungstenite::tungstenite::Message::Text(reply.to_string().into());
                if let Some(gate) = gate {
                    let replies = replies.clone();
                    tokio::spawn(async move {
                        gate.notified().await;
                        let _ = replies.send(frame);
                    });
                } else if replies.send(frame).is_err() {
                    break;
                }
            }
            sender.abort();
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
        store.lock().unwrap().topology_probe = Some(deps.topology.clone());
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
    /// Park `id` on one external pending call (no hook hold, no child): the
    /// shape nothing but its external resolver can confirm.
    fn park_on_external_call(&self, id: &str) {
        let mut store = self.store.lock().unwrap();
        let mut turn = store.state("harness_turn", id);
        turn["status"] = json!("awaiting_functions");
        turn["calls"] = json!({"ext-1": {"state": "pending", "function_id": "slow::external"}});
        store.put("harness_turn", id, turn);
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
async fn erase_does_not_hold_the_process_topology_lock_across_session_delete() {
    let stack = Stack::new("running").await;
    let accepted = stack.request("child2").await;
    let done = stack.run(&accepted.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed);
    let store = stack.store.lock().unwrap();
    // One probe per erased member (grandchild1, child2). Holding the lock
    // there would stall every send, spawn and binding in the process.
    assert_eq!(store.topology_held_on_delete.len(), 2);
    assert!(
        store.topology_held_on_delete.iter().all(|held| !held),
        "topology lock held during session::delete: {:?}",
        store.topology_held_on_delete
    );
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
            op["links"] = json!([["child2", null], ["grandchild1", "child2"]]);
            store.put(deletion::OPERATIONS, &accepted.operation_id, op);
        }
        let result = stack.run(&accepted.operation_id).await;
        assert_eq!(result.status, DeletionStatus::Completed, "{result:?}");
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
        serde_json::from_value::<DeleteRequest>(json!({"session_id":"child2"})).unwrap(),
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
    assert_eq!(
        failed.failure_code,
        Some(deletion::DeletionFailureCode::Blocked)
    );
    assert!(failed.force_eligible);
    assert_eq!(
        failed.blockers[0].function_id.as_deref(),
        Some("slow::work")
    );
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
        let expected_unknown =
            if stack.store.lock().unwrap().delete_reply.as_ref().unwrap()["deleted"].is_boolean() {
                Vec::<String>::new()
            } else {
                vec!["grandchild1".into()]
            };
        assert_eq!(failed.unconfirmed_session_ids, expected_unknown);
        let caught_up = deletion::status(
            &stack.deps,
            StatusRequest {
                operation_id: failed.operation_id.clone(),
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(caught_up.unconfirmed_session_ids, expected_unknown);
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
async fn missing_session_after_lost_delete_ack_with_explicit_intent_is_an_idempotent_success() {
    let stack = Stack::new("completed").await;
    let op = stack.request("child2").await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut value = store.state(deletion::OPERATIONS, &op.operation_id);
        value["planned"] = json!(true);
        value["members"] = json!(["child2", "grandchild1"]);
        value["parent"] = json!("parent");
        value["links"] = json!([["child2", null], ["grandchild1", "child2"]]);
        value["erasing"] = json!("grandchild1");
        store.put(deletion::GUARDS, "grandchild1", json!(op.operation_id));
        store.put(deletion::OPERATIONS, &op.operation_id, value);
        store.sessions.remove("grandchild1");
    }
    let done = stack.run(&op.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
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
        serde_json::from_value::<DeleteRequest>(json!({"session_id":"child2"})).unwrap()
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
        serde_json::from_value::<DeleteRequest>(json!({"session_id":"child2"})).unwrap()
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

fn turn(stack: &Stack, id: &str) -> Value {
    stack.store.lock().unwrap().state("harness_turn", id)
}

async fn ordinary_stop(stack: &Stack, id: &str) -> harness::functions::stop::StopResponse {
    harness::functions::stop::handle(
        &stack.deps,
        harness::functions::stop::StopRequest {
            session_id: id.into(),
            turn_id: None,
        },
    )
    .await
    .expect("an ordinary stop never fails on an unconfirmed cancellation")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_external_result_while_tombstoned_lets_the_retry_delete_the_subtree() {
    let stack = Stack::new("running").await;
    stack.park_on_external_call("grandchild1");
    let accepted = stack.request("child2").await;
    let failed = stack.run(&accepted.operation_id).await;
    assert_eq!(failed.status, DeletionStatus::Failed);
    assert!(
        failed
            .error
            .as_deref()
            .is_some_and(|e| e.contains("data retained")),
        "{failed:?}"
    );
    assert_eq!(stack.store.lock().unwrap().sessions.len(), 4);
    assert_eq!(turn(&stack, "grandchild1")["status"], "awaiting_functions");

    // The external result arrives while the subtree is tombstoned.
    let resolved = harness::functions::function_resolve::handle(
        &stack.deps,
        serde_json::from_value(json!({
            "session_id": "grandchild1",
            "turn_id": "t_grandchild1",
            "function_call_id": "ext-1",
            "content": [{"type": "text", "text": "late result"}]
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(resolved.resolved);
    assert!(!resolved.turn_resumed);
    let settled = turn(&stack, "grandchild1");
    assert_eq!(settled["status"], "cancelled");
    // Settled, not left pending: the finished record drops a done call
    // without a child (MOT-5166).
    assert!(settled["calls"].get("ext-1").is_none(), "{settled}");
    {
        let store = stack.store.lock().unwrap();
        // Consumed without a model-visible result or a resumed step.
        assert!(!store
            .messages
            .get("grandchild1")
            .is_some_and(|m| m.iter().any(|e| e.to_string().contains("late result"))));
        assert!(!store
            .calls
            .iter()
            .any(|(f, p)| f == "harness::turn" && p["session_id"] == "grandchild1"));
    }

    let retry = stack.request("child2").await;
    assert_eq!(retry.attempt, 2);
    let done = stack.run(&retry.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    assert_eq!(done.deleted_session_ids, vec!["grandchild1", "child2"]);
    let store = stack.store.lock().unwrap();
    assert_eq!(
        store
            .state
            .iter()
            .filter(|((scope, _), row)| scope == "harness_queue" && row["session_id"] == "parent")
            .count(),
        1,
        "the parent is notified exactly once"
    );
}

/// A hook-held scoped call released after `filesystem_boundary` widened to
/// `configured_roots` still runs under the `workspace` boundary its holder
/// reviewed (MOT-5167).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_released_call_keeps_the_boundary_its_holder_reviewed() {
    let stack = Stack::new("completed").await;
    *stack.deps.config.write().await = Arc::new(WorkerConfig {
        session_timeout_ms: 2_000,
        filesystem_boundary: harness::filesystem_scope::BoundaryMode::ConfiguredRoots,
        ..WorkerConfig::default()
    });
    {
        let mut store = stack.store.lock().unwrap();
        let mut turn = store.state("harness_turn", "child1");
        turn["status"] = json!("awaiting_functions");
        turn["options"]["metadata"] = json!({"fs_scope": {"root": "/w"}});
        turn["calls"] = json!({"held-1": {
            "state": "pending",
            "function_id": "shell::exec",
            // Not bound: the release resumes an empty chain.
            "held_by": "gone::gate",
            "held_arguments": {
                "command": "ls",
                "fs_scope": {"root": "/w", "grants": [], "boundary": "workspace"}
            }
        }});
        store.put("harness_turn", "child1", turn);
    }
    let resolved = harness::functions::function_resolve::handle(
        &stack.deps,
        serde_json::from_value(json!({
            "session_id": "child1",
            "turn_id": "t_child1",
            "function_call_id": "held-1",
            "action": "execute"
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(resolved.resolved);
    let store = stack.store.lock().unwrap();
    let (_, payload) = store
        .calls
        .iter()
        .find(|(f, _)| f == "shell::exec")
        .expect("the released call is dispatched");
    assert_eq!(payload["fs_scope"]["root"], "/w", "{payload}");
    assert_eq!(payload["fs_scope"]["boundary"], "workspace", "{payload}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_stop_persists_abort_on_router_failure_but_deletion_fails_closed() {
    for code in ["test_error", "function_not_found"] {
        let stack = Stack::new("running").await;
        stack.set_status("grandchild1", "running");
        {
            let mut store = stack.store.lock().unwrap();
            let mut row = store.state("harness_turn", "grandchild1");
            row["stream_request_id"] = json!("stream");
            store.put("harness_turn", "grandchild1", row);
            store.codes.insert("router::abort".into(), code.into());
        }
        assert!(
            ordinary_stop(&stack, "grandchild1").await.stopping,
            "{code}"
        );
        assert_eq!(turn(&stack, "grandchild1")["abort"], true, "{code}");

        let accepted = stack.request("child2").await;
        let failed = stack.run(&accepted.operation_id).await;
        assert_eq!(failed.status, DeletionStatus::Failed, "{code}");
        assert!(
            failed
                .error
                .as_deref()
                .is_some_and(|e| e.contains("router::abort")),
            "{code}: {failed:?}"
        );
        assert_eq!(stack.store.lock().unwrap().sessions.len(), 4, "{code}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_stop_of_unconfirmed_external_call_reports_stopping_with_abort() {
    let stack = Stack::new("running").await;
    stack.park_on_external_call("grandchild1");
    assert!(ordinary_stop(&stack, "grandchild1").await.stopping);
    let parked = turn(&stack, "grandchild1");
    assert_eq!(parked["abort"], true);
    // Not finalized: the external call's outcome is still unknown.
    assert_eq!(parked["status"], "awaiting_functions");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_skips_malformed_rows_and_continues_after_an_enqueue_failure() {
    let stack = Stack::new("completed").await;
    let child2 = stack.request("child2").await;
    let child1 = stack.request("child1").await;
    // Rows are listed in key order: both malformed rows sort first, then the
    // operation whose enqueue fails, then the one that must still be enqueued.
    let (failing, failing_session, other) = if child1.operation_id < child2.operation_id {
        (&child1, "child1", &child2)
    } else {
        (&child2, "child2", &child1)
    };
    {
        let mut store = stack.store.lock().unwrap();
        let mut zero = store.state(deletion::OPERATIONS, &child2.operation_id);
        zero["snapshot"]["operation_id"] = json!("delete_!zero");
        zero["snapshot"]["attempt"] = json!(0);
        store.put(deletion::OPERATIONS, "delete_!zero", zero);
        store.put(
            deletion::OPERATIONS,
            "delete_!garbage",
            json!({"snapshot": "not an operation"}),
        );
        store.fail_enqueue.insert(failing.operation_id.clone());
        store.calls.clear();
    }
    deletion::recover(&stack.deps)
        .await
        .expect("bad rows and enqueue failures must not abort boot recovery");
    let enqueued = |store: &Store| -> Vec<String> {
        store
            .calls
            .iter()
            .filter(|(f, _)| f == deletion::RUN_ID)
            .filter_map(|(_, p)| p["operation_id"].as_str().map(str::to_string))
            .collect()
    };
    {
        let store = stack.store.lock().unwrap();
        assert_eq!(
            enqueued(&store),
            vec![failing.operation_id.clone(), other.operation_id.clone()]
        );
        // The malformed row is left untouched: its operation stays fail-closed.
        assert_eq!(
            store.state(deletion::OPERATIONS, "delete_!zero")["snapshot"]["attempt"],
            0
        );
    }
    // A client retry of the pending command re-enqueues the stranded operation.
    {
        let mut store = stack.store.lock().unwrap();
        store.fail_enqueue.clear();
        store.calls.clear();
    }
    let pending = stack.request(failing_session).await;
    assert_eq!(pending.status, DeletionStatus::Deleting);
    assert_eq!(pending.attempt, 1);
    assert_eq!(
        enqueued(&stack.store.lock().unwrap()),
        vec![failing.operation_id.clone()]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_dispatch_witness_follows_the_structured_outcome_not_error_text() {
    let stack = Stack::new("completed").await;
    stack
        .store
        .lock()
        .unwrap()
        .codes
        .insert("ext::stopped".into(), "invocation_stopped".into());
    let engine = stack.deps.engine().await;
    let policy = harness::policy::CompiledPolicy::from(None);
    // The first target's failure text mentions a connection timeout, but the
    // target answered: its outcome is known and the witness is released.
    for (function, keeps_witness) in [("ext::connection-timeout", false), ("ext::stopped", true)] {
        let result = harness::functions::subscribe::invoke(
            &stack.deps,
            &engine,
            &policy,
            function,
            &json!({}),
            "child1",
            false,
            None,
        )
        .await;
        assert!(result.is_error, "{function}");
        let witnesses = stack
            .store
            .lock()
            .unwrap()
            .state
            .iter()
            .filter(|((scope, _), row)| {
                scope == deletion::DISPATCHES && row["function_id"] == function
            })
            .count();
        assert_eq!(witnesses, usize::from(keeps_witness), "{function}");
    }
}

/// Tombstone reads (`state::get` on the guard scope) the fixture has served.
fn guard_reads(store: &Store) -> usize {
    store
        .calls
        .iter()
        .filter(|(function, data)| {
            function.ends_with("state::get") && data["scope"] == deletion::GUARDS
        })
        .count()
}

/// Witness rows still present; a cleared witness is stored as `null`.
fn dispatch_witnesses(store: &Store) -> Vec<Value> {
    store
        .state
        .iter()
        .filter(|((scope, _), row)| scope == deletion::DISPATCHES && !row.is_null())
        .map(|(_, row)| row.clone())
        .collect()
}

/// The RPCs the fixture served on the witness scope, in order.
fn witness_rpcs(store: &Store) -> Vec<String> {
    store
        .calls
        .iter()
        .filter(|(_, data)| data["scope"] == deletion::DISPATCHES)
        .map(|(function, _)| function.clone())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_dispatch_takes_no_process_wide_lock_and_reuses_a_live_answer() {
    let stack = Stack::new("completed").await;
    let engine = stack.deps.engine().await;
    let policy = harness::policy::CompiledPolicy::from(None);
    stack.store.lock().unwrap().calls.clear();
    // Another session's admission holds topology for the whole exchange: a
    // dispatch must not queue behind it.
    let held = stack.deps.topology.clone().lock_owned().await;
    let mut reads = Vec::new();
    for _ in 0..2 {
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            harness::functions::subscribe::invoke(
                &stack.deps,
                &engine,
                &policy,
                "ext::probe",
                &json!({}),
                "child1",
                false,
                None,
            ),
        )
        .await
        .expect("dispatch must not wait on the process-wide topology lock");
        // The fixture answers an unknown target with an error: it was reached.
        assert!(result.is_error);
        reads.push(guard_reads(&stack.store.lock().unwrap()));
    }
    drop(held);
    let store = stack.store.lock().unwrap();
    // The first dispatch walks child1 -> parent; the second reuses that answer.
    assert_eq!(reads, vec![2, 2]);
    assert_eq!(
        store
            .calls
            .iter()
            .filter(|(f, _)| f == "ext::probe")
            .count(),
        2
    );
    // Each dispatch writes its witness with one CAS and clears it with one.
    assert_eq!(
        witness_rpcs(&store),
        vec!["harness::state::compare-and-set"; 4]
    );
    // Both replies confirmed their calls, so no witness outlives them.
    let witnesses = dispatch_witnesses(&store);
    assert!(witnesses.is_empty(), "{witnesses:#?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tombstone_after_a_live_answer_refuses_dispatch_before_writing_a_witness() {
    let stack = Stack::new("completed").await;
    let engine = stack.deps.engine().await;
    let policy = harness::policy::CompiledPolicy::from(None);
    let args = json!({});
    let dispatch = |function: &'static str| {
        harness::functions::subscribe::invoke(
            &stack.deps,
            &engine,
            &policy,
            function,
            &args,
            "child1",
            false,
            None,
        )
    };
    // child1 is remembered live...
    assert!(dispatch("ext::before").await.is_error);
    // ...then this process tombstones its parent, well within the memo's TTL.
    let accepted = stack.request("parent").await;
    assert_eq!(accepted.status, DeletionStatus::Deleting);
    stack.store.lock().unwrap().calls.clear();

    let refused = dispatch("ext::after").await;
    assert!(refused.is_error);
    let message = refused.details["error"]["message"]
        .as_str()
        .unwrap_or_default();
    assert!(message.contains("tombstoned"), "{message}");
    let store = stack.store.lock().unwrap();
    // The guard write invalidated the memo: the lineage was read again...
    assert!(guard_reads(&store) > 0);
    // ...the target was never invoked and no witness was written at all.
    assert!(witness_rpcs(&store).is_empty());
    assert!(!store.calls.iter().any(|(f, _)| f == "ext::after"));
    let witnesses = dispatch_witnesses(&store);
    assert!(witnesses.is_empty(), "{witnesses:#?}");
}

/// The orphan redrive's view of the turn scope is process-wide: tests that
/// read or refresh it run one at a time.
static TURN_VIEW_TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// `harness_turn` keys read with `state::get` since the calls were cleared.
fn turn_gets(store: &Store) -> Vec<String> {
    store
        .calls
        .iter()
        .filter(|(f, data)| f == "state::get" && data["scope"] == "harness_turn")
        .map(|(_, data)| data["key"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// The orphan redrive reads only the turn records that changed since its last
/// pass, and the pending sweep's full read refreshes that view: a record
/// rewritten behind this process (a state-store rollback, a console edit) is
/// redriven by the next sweep, not only after a restart.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orphan_redrive_reads_only_changed_turns_and_the_sweep_refreshes_its_view() {
    use harness::functions::sweep_pending::{self, SweepEvent};
    use harness::inflight::redrive_orphans;
    let _view = TURN_VIEW_TESTS.lock().await;
    let stack = Stack::new("completed").await;
    // Own keys only: the other tests' writes mark the fixture's ids.
    stack
        .store
        .lock()
        .unwrap()
        .state
        .retain(|(scope, _), _| scope != "harness_turn");
    stack.session("rd_done", None, "completed");
    stack.session("rd_parked", None, "awaiting_functions");
    let pass = || async {
        stack.store.lock().unwrap().calls.clear();
        let redriven = redrive_orphans(&stack.deps).await.unwrap();
        (redriven, turn_gets(&stack.store.lock().unwrap()))
    };
    assert_eq!(
        pass().await,
        (0, vec!["rd_done".into(), "rd_parked".into()])
    );
    // Nothing changed: keys only.
    assert_eq!(pass().await, (0, vec![]));
    // Rolled back to Running behind this process: the incremental pass cannot
    // see it...
    stack.set_status("rd_done", "running");
    assert_eq!(pass().await, (0, vec![]));
    // ...the sweep's one full read refreshes the view, and its redrive pass
    // then reads (and re-checks) only the orphan and re-enqueues it.
    stack.store.lock().unwrap().calls.clear();
    let swept = sweep_pending::handle(&stack.deps, SweepEvent::default())
        .await
        .unwrap();
    assert_eq!(swept.redriven, 1);
    assert_eq!(
        turn_gets(&stack.store.lock().unwrap()),
        ["rd_done", "rd_parked", "rd_done", "rd_done"]
    );
}

/// A finished turn's record keeps only what a finished turn is read for
/// (MOT-5166): open calls and calls with a child stay; done calls without
/// one, the failure counts, the watermark and the stream id go.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finalized_turn_records_drop_what_only_a_running_turn_reads() {
    let stack = Stack::new("awaiting_functions").await;
    stack.set_status("grandchild1", "running");
    for id in ["parent", "child1", "grandchild1"] {
        let mut store = stack.store.lock().unwrap();
        let mut row = store.state("harness_turn", id);
        row["calls"] = json!({
            "done": {"state": "done", "function_id": "x::y"},
            "spawn": {"state": "done", "function_id": "harness::spawn",
                "child_session_id": "s_c", "child_turn_id": "t_c"}
        });
        row["failed_calls"] = json!({"k": {"error_digest": "e", "count": 2}});
        row["watermark_entry_id"] = json!("e_w");
        row["stream_request_id"] = json!("req");
        store.put("harness_turn", id, row);
    }
    // finalize_completed: a step that finds its step cap spent completes the
    // turn without a generation. First: stopping the parent cascades to it.
    {
        let mut store = stack.store.lock().unwrap();
        let mut row = store.state("harness_turn", "grandchild1");
        row["turn_count"] = json!(16);
        store.put("harness_turn", "grandchild1", row);
    }
    let payload = serde_json::from_value(
        json!({"session_id":"grandchild1","turn_id":"t_grandchild1","step":0,"depth":0}),
    )
    .unwrap();
    harness::turn_loop::run_step(&stack.deps, payload)
        .await
        .unwrap();
    // finalize_cancelled: a stop on a parked turn with no external call.
    ordinary_stop(&stack, "parent").await;
    // finalize_failed: an unexpected step error.
    harness::turn_loop::fail_turn(&stack.deps, "child1", "t_child1", "boom")
        .await
        .unwrap();
    for (id, status) in [
        ("parent", "cancelled"),
        ("child1", "failed"),
        ("grandchild1", "completed"),
    ] {
        let row = turn(&stack, id);
        assert_eq!(row["status"], status, "{id}");
        let calls: Vec<&String> = row["calls"].as_object().unwrap().keys().collect();
        assert_eq!(calls, ["spawn"], "{id}");
        for field in ["failed_calls", "watermark_entry_id", "stream_request_id"] {
            assert!(row.get(field).is_none(), "{id}: {field} = {}", row[field]);
        }
    }
}

async fn session_deleted(deps: &Deps, event: Value) {
    harness::functions::on_session_deleted::handle(deps, serde_json::from_value(event).unwrap())
        .await
        .unwrap();
}

/// `session::deleted` purges the session's turn record with its other rows
/// (MOT-5166); other sessions' records stay.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_deleted_removes_the_turn_record() {
    let stack = Stack::new("completed").await;
    session_deleted(&stack.deps, json!({"session_id": "child2", "timestamp": 1})).await;
    let store = stack.store.lock().unwrap();
    assert!(store.calls.iter().any(|(f, data)| f == "state::delete"
        && data["scope"] == "harness_turn"
        && data["key"] == "child2"));
    assert!(store.state("harness_turn", "child2").is_null());
    assert!(!store.state("harness_turn", "grandchild1").is_null());
}

/// A step holds its record in memory and writes it back when it ends. The
/// purge waits that step out, so it deletes the step's last write instead of
/// the step re-creating the record it just deleted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_deleted_waits_out_a_running_step() {
    let stack = Stack::new("completed").await;
    let held = turn(&stack, "child1");
    let step = stack.deps.turn_activity.guard("child1").await;
    let deps = stack.deps.clone();
    let purge = tokio::spawn(async move {
        session_deleted(&deps, json!({"session_id": "child1", "timestamp": 1})).await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        turn(&stack, "child1")["turn_id"],
        "t_child1",
        "deleted under a running step"
    );
    // The step's final put_turn, then the step ends.
    stack
        .store
        .lock()
        .unwrap()
        .put("harness_turn", "child1", held);
    drop(step);
    tokio::time::timeout(Duration::from_secs(5), purge)
        .await
        .unwrap()
        .unwrap();
    assert!(turn(&stack, "child1").is_null());
}

/// A finished record written before prompt refs: its frozen texts inline,
/// one done call a finished record no longer keeps.
fn inline_prompt_turn(stack: &Stack, id: &str, prompt: &str, index: &str) {
    stack.session(id, None, "completed");
    let mut store = stack.store.lock().unwrap();
    let mut row = store.state("harness_turn", id);
    row["options"]["system_prompt"] = json!(prompt);
    row["options"]["skill_context"] = json!({"baseline": index});
    row["calls"] = json!({"done": {"state": "done", "function_id": "x::y"}});
    store.put("harness_turn", id, row);
}

/// `harness::turn_compaction::compact` over a fresh listing of the store.
async fn compact(stack: &Stack, now: i64) -> harness::turn_compaction::CompactReport {
    let listing = harness::state::list_turns(&stack.deps.iii, 2_000)
        .await
        .unwrap();
    harness::turn_compaction::compact(&stack.deps, &listing, now).await
}

fn turn_writes(store: &Store) -> usize {
    store
        .calls
        .iter()
        .filter(|(f, data)| f == "state::set" && data["scope"] == "harness_turn")
        .count()
}

const DAY_MS: i64 = 86_400_000;

// Prompt texts are unique per test: the harness remembers which bodies it
// stored process-wide, and each test has its own store.

/// A finished record that still holds its prompt inline is rewritten once:
/// bodies to `harness_prompt`, refs in the record, slimmed, `updated_at`
/// kept. The next pass finds nothing inline and writes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compact_converts_inline_terminal_records_once() {
    let stack = Stack::new("completed").await;
    let (prompt, index) = ("compact_converts prompt", "compact_converts index");
    inline_prompt_turn(&stack, "cc_old", prompt, index);
    let now = harness::types::message::AgentMessage::now_ms();
    assert_eq!(compact(&stack, now).await.converted, 1);

    let row = turn(&stack, "cc_old");
    assert_eq!(row["updated_at"], 1);
    assert_eq!(row["calls"], json!({}));
    {
        let store = stack.store.lock().unwrap();
        for (field, text) in [
            (&row["options"]["system_prompt"], prompt),
            (&row["options"]["skill_context"]["baseline"], index),
        ] {
            let digest = field["$ref"].as_str().expect("a ref, not the text");
            assert!(digest.starts_with("sha256:"), "{digest}");
            assert_eq!(store.state("harness_prompt", digest)["body"], text);
        }
    }
    let read = harness::state::get_turn(&stack.deps.iii, "cc_old", 2_000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.options.system_prompt.as_deref(), Some(prompt));
    assert_eq!(
        read.options.skill_context.unwrap().baseline.as_deref(),
        Some(index)
    );

    stack.store.lock().unwrap().calls.clear();
    let again = compact(&stack, now).await;
    assert_eq!((again.converted, again.prompts_collected), (0, 0));
    assert_eq!(turn_writes(&stack.store.lock().unwrap()), 0);
}

/// Conversion leaves a turn that is still running, one whose step executes
/// here, and one rewritten since the listing (it re-reads under the session's
/// guards and requires the listed `turn_id` and `updated_at`). A step holds
/// `turn_activity` for its whole run: conversion skips its session without
/// waiting for the step to end (boot compaction runs in the redrive loop).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compact_skips_running_and_changed_records() {
    let stack = Stack::new("completed").await;
    let prompt = "compact_skips prompt";
    for id in ["cs_running", "cs_inflight", "cs_changed"] {
        inline_prompt_turn(&stack, id, prompt, "compact_skips index");
    }
    stack.set_status("cs_running", "running");
    let listing = harness::state::list_turns(&stack.deps.iii, 2_000)
        .await
        .unwrap();
    {
        let mut store = stack.store.lock().unwrap();
        let mut row = store.state("harness_turn", "cs_changed");
        row["updated_at"] = json!(2);
        store.put("harness_turn", "cs_changed", row);
        store.calls.clear();
    }
    let _step = stack.deps.inflight.enter("cs_inflight");
    let _activity = stack.deps.turn_activity.guard("cs_inflight").await;
    let now = harness::types::message::AgentMessage::now_ms();
    let report = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        harness::turn_compaction::compact(&stack.deps, &listing, now),
    )
    .await
    .expect("conversion must not wait for a running step");
    assert_eq!(report.converted, 0);
    assert_eq!(turn_writes(&stack.store.lock().unwrap()), 0);
    for id in ["cs_running", "cs_inflight", "cs_changed"] {
        assert_eq!(turn(&stack, id)["options"]["system_prompt"], prompt, "{id}");
    }
}

/// A body goes only when no record references it and it is older than the
/// grace period, which covers a send that stored a body but has not yet
/// written the record that references it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compact_collects_only_old_unreferenced_bodies() {
    let stack = Stack::new("completed").await;
    let now = harness::types::message::AgentMessage::now_ms();
    {
        let mut store = stack.store.lock().unwrap();
        let mut row = store.state("harness_turn", "child2");
        row["options"]["system_prompt"] = json!({"$ref": "sha256:cb_a"});
        store.put("harness_turn", "child2", row);
        for (digest, created_at) in [
            ("sha256:cb_a", now - 2 * DAY_MS),
            ("sha256:cb_b", now - 2 * DAY_MS),
            ("sha256:cb_c", now - DAY_MS / 24),
        ] {
            store.put(
                "harness_prompt",
                digest,
                json!({"body": digest, "created_at": created_at}),
            );
        }
    }
    let report = compact(&stack, now).await;
    assert_eq!((report.converted, report.prompts_collected), (0, 1));
    let store = stack.store.lock().unwrap();
    assert!(store.state("harness_prompt", "sha256:cb_b").is_null());
    for kept in ["sha256:cb_a", "sha256:cb_c"] {
        assert_eq!(store.state("harness_prompt", kept)["body"], kept);
    }
}

/// A record this build cannot parse (a build with a newer record shape wrote
/// it) is left out of the listing, yet its refs are in use: once that build
/// runs again, a body collected here would make the session's every read a
/// "missing prompt body". The collector keeps every body it names.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compact_keeps_the_bodies_of_a_record_it_cannot_parse() {
    let stack = Stack::new("completed").await;
    let now = harness::types::message::AgentMessage::now_ms();
    stack.session("cu_newer", None, "completed");
    {
        let mut store = stack.store.lock().unwrap();
        let mut row = store.state("harness_turn", "cu_newer");
        row["status"] = json!("a_status_from_a_newer_build");
        row["options"]["system_prompt"] = json!({"$ref": "sha256:cu_used"});
        store.put("harness_turn", "cu_newer", row);
        for digest in ["sha256:cu_used", "sha256:cu_unused"] {
            store.put(
                "harness_prompt",
                digest,
                json!({"body": digest, "created_at": now - 2 * DAY_MS}),
            );
        }
    }
    let report = compact(&stack, now).await;
    assert_eq!(report.prompts_collected, 1);
    let store = stack.store.lock().unwrap();
    assert!(store.state("harness_prompt", "sha256:cu_unused").is_null());
    assert_eq!(
        store.state("harness_prompt", "sha256:cu_used")["body"],
        "sha256:cu_used"
    );
}

/// The pending sweep converts and collects from the one full read of the
/// turn scope it already makes; the conversion re-reads only the record it
/// rewrites.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_sweep_compacts_from_its_one_full_read() {
    use harness::functions::sweep_pending::{self, SweepEvent};
    let _view = TURN_VIEW_TESTS.lock().await;
    let stack = Stack::new("completed").await;
    // Own keys only: the other tests' writes mark the fixture's ids.
    stack
        .store
        .lock()
        .unwrap()
        .state
        .retain(|(scope, _), _| scope != "harness_turn");
    stack.session("sw_done", None, "completed");
    inline_prompt_turn(&stack, "sw_old", "sweep prompt", "sweep index");
    {
        let mut store = stack.store.lock().unwrap();
        store.put(
            "harness_prompt",
            "sha256:sw_unused",
            json!({"body": "unused", "created_at": 1}),
        );
        store.calls.clear();
    }
    let swept = sweep_pending::handle(&stack.deps, SweepEvent::default())
        .await
        .unwrap();
    assert_eq!((swept.converted, swept.prompts_collected), (1, 1));
    assert_eq!(
        turn_gets(&stack.store.lock().unwrap()),
        ["sw_done", "sw_old", "sw_old"]
    );
    assert!(turn(&stack, "sw_old")["options"]["system_prompt"]["$ref"].is_string());
}

/// Point `id`'s record at a prompt body the store does not have.
fn lose_prompt_body(stack: &Stack, id: &str) {
    let mut store = stack.store.lock().unwrap();
    let mut row = store.state("harness_turn", id);
    row["options"]["system_prompt"] = json!({"$ref": format!("sha256:lost_{id}")});
    store.put("harness_turn", id, row);
}

/// A record whose prompt body is gone is redriven like any orphan (its step
/// then fails the turn instead of leaving it Running), and does not end the
/// pass for the orphans after it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lost_prompt_body_does_not_stop_the_orphan_redrive() {
    let _view = TURN_VIEW_TESTS.lock().await;
    let stack = Stack::new("completed").await;
    // Own keys only: the other tests' writes mark the fixture's ids.
    stack
        .store
        .lock()
        .unwrap()
        .state
        .retain(|(scope, _), _| scope != "harness_turn");
    stack.session("lb_a_lost", None, "running");
    stack.session("lb_b_orphan", None, "running");
    lose_prompt_body(&stack, "lb_a_lost");
    let redriven = harness::inflight::redrive_orphans(&stack.deps)
        .await
        .unwrap();
    assert_eq!(redriven, 2);
    let store = stack.store.lock().unwrap();
    let mut enqueued: Vec<&Value> = store
        .calls
        .iter()
        .filter(|(f, _)| f == "harness::turn")
        .map(|(_, data)| &data["session_id"])
        .collect();
    enqueued.sort_by_key(|id| id.to_string());
    assert_eq!(enqueued, [&json!("lb_a_lost"), &json!("lb_b_orphan")]);
}

/// Stop and delete-session-tree read only a record's status, turn and calls,
/// so a session whose prompt body is gone can still be stopped and deleted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_and_delete_work_without_the_prompt_body() {
    let stack = Stack::new("running").await;
    stack.set_status("grandchild1", "awaiting_functions");
    for id in ["child2", "grandchild1"] {
        lose_prompt_body(&stack, id);
    }
    let stopped = harness::functions::stop::handle(
        &stack.deps,
        harness::functions::stop::StopRequest {
            session_id: "grandchild1".into(),
            turn_id: Some("t_grandchild1".into()),
        },
    )
    .await
    .unwrap();
    assert!(stopped.stopping);
    let row = turn(&stack, "grandchild1");
    assert_eq!(row["status"], "cancelled");
    assert_eq!(
        row["options"]["system_prompt"]["$ref"], "sha256:lost_grandchild1",
        "the write-back keeps the ref"
    );
    let accepted = stack.request("child2").await;
    let done = stack.run(&accepted.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    assert_eq!(done.deleted_session_ids, vec!["grandchild1", "child2"]);
}

fn step(id: &str) -> harness::turn_loop::TurnStepPayload {
    serde_json::from_value(json!({"session_id":id,"turn_id":format!("t_{id}"),"step":0,"depth":0}))
        .unwrap()
}

/// A stopped step and a failed step finalize from the stored record: neither
/// needs the prompt body, so a session whose body is gone still ends.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopped_and_failed_steps_finalize_without_the_prompt_body() {
    let stack = Stack::new("completed").await;
    stack.set_status("grandchild1", "running");
    for id in ["child1", "grandchild1"] {
        lose_prompt_body(&stack, id);
    }
    assert!(ordinary_stop(&stack, "child1").await.stopping);
    let stopped = harness::turn_loop::run_step(&stack.deps, step("child1"))
        .await
        .unwrap();
    assert_eq!(turn(&stack, "child1")["status"], "cancelled", "{stopped:?}");

    // Not stopped: generating needs the body, so the step fails and the
    // failure finalizes.
    let error = harness::turn_loop::run_step(&stack.deps, step("grandchild1"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("missing prompt body"), "{error}");
    harness::turn_loop::fail_turn(
        &stack.deps,
        "grandchild1",
        "t_grandchild1",
        &error.to_string(),
    )
    .await
    .unwrap();
    assert_eq!(turn(&stack, "grandchild1")["status"], "failed");
    for id in ["child1", "grandchild1"] {
        assert_eq!(
            turn(&stack, id)["options"]["system_prompt"]["$ref"],
            format!("sha256:lost_{id}"),
            "{id}: the write-back keeps the ref"
        );
    }
}

/// `harness::status` reads no prompt text.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_reports_a_session_without_its_prompt_body() {
    let stack = Stack::new("completed").await;
    lose_prompt_body(&stack, "parent");
    let report = harness::functions::status::handle(
        &stack.deps,
        harness::functions::status::StatusRequest {
            session_id: "parent".into(),
            verbose: true,
        },
    )
    .await
    .unwrap()
    .expect("a report");
    assert_eq!(report.turn_id.as_deref(), Some("t_parent"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_overlap_diagnostic_errors_release_only_own_unplanned_root() {
    for case in [
        "missing",
        "malformed",
        "nonstring",
        "read",
        "release",
        "foreign-root",
    ] {
        let stack = Stack::new("completed").await;
        let accepted = stack.request("parent").await;
        {
            let mut store = stack.store.lock().unwrap();
            assert_eq!(
                store.state(deletion::GUARDS, "parent"),
                json!(accepted.operation_id)
            );
            store.put(
                deletion::GUARDS,
                "grandchild1",
                if case == "nonstring" {
                    json!(42)
                } else {
                    json!("delete_foreign")
                },
            );
            if case == "malformed" {
                store.put(
                    deletion::OPERATIONS,
                    "delete_foreign",
                    json!({"snapshot":{"bad":true}}),
                );
            }
            if case == "read" {
                store.fail_state_read =
                    Some((deletion::OPERATIONS.into(), "delete_foreign".into()));
            }
            store.fail_root_release = case == "release";
            if case == "foreign-root" {
                store.put(deletion::GUARDS, "parent", json!("foreign-parent"));
            }
        }
        let failed = stack.run(&accepted.operation_id).await;
        assert_eq!(failed.status, DeletionStatus::Failed, "{case}");
        assert!(failed.existing_deletion.is_none(), "{case}");
        assert!(!failed.force_eligible, "{case}");
        {
            let store = stack.store.lock().unwrap();
            let expected = match case {
                "release" => json!(accepted.operation_id),
                "foreign-root" => json!("foreign-parent"),
                _ => Value::Null,
            };
            assert_eq!(store.state(deletion::GUARDS, "parent"), expected, "{case}");
            assert!(store.state(deletion::GUARDS, "child1").is_null(), "{case}");
            assert!(!store.calls.iter().any(|(f, _)| f == "session::delete"));
            if case == "read" {
                let release = store
                    .calls
                    .iter()
                    .position(|(f, d)| {
                        f == "harness::state::compare-and-set"
                            && d["scope"] == deletion::GUARDS
                            && d["key"] == "parent"
                            && d["value"].is_null()
                    })
                    .unwrap();
                let read = store
                    .calls
                    .iter()
                    .position(|(f, d)| {
                        f == "harness::state::get"
                            && d["scope"] == deletion::OPERATIONS
                            && d["key"] == "delete_foreign"
                    })
                    .unwrap();
                assert!(release < read);
            }
        }
        if !["release", "foreign-root"].contains(&case) {
            stack.set_status("child1", "completed");
            let sibling = stack.request("child1").await;
            assert_eq!(
                sibling.status,
                DeletionStatus::Deleting,
                "{case}: {sibling:?}"
            );
            assert_eq!(
                stack.run(&sibling.operation_id).await.status,
                DeletionStatus::Completed
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_overlap_identity_requires_canonical_id_and_own_root_guard() {
    for case in ["canonical", "root-guard", "root-read"] {
        let stack = Stack::new("running").await;
        stack.store.lock().unwrap().put(
            deletion::DISPATCHES,
            "legacy",
            json!({"session_id":"grandchild1","function_id":"browser::fetch"}),
        );
        let accepted = stack.request("child2").await;
        let blocked = stack.run(&accepted.operation_id).await;
        assert!(blocked.force_eligible);
        {
            let mut store = stack.store.lock().unwrap();
            if case == "canonical" {
                let mut row = store.state(deletion::OPERATIONS, &blocked.operation_id);
                row["snapshot"]["operation_id"] = json!("noncanonical");
                store.put(deletion::OPERATIONS, "noncanonical", row);
                for id in ["child2", "grandchild1"] {
                    store.put(deletion::GUARDS, id, json!("noncanonical"));
                }
            } else {
                // Descendant remains owned but selected root no longer corroborates.
                store.put(deletion::GUARDS, "child2", Value::Null);
                if case == "root-read" {
                    store.fail_state_read = Some((deletion::GUARDS.into(), "child2".into()));
                }
            }
        }
        let failed = stack.request("parent").await;
        assert_eq!(failed.status, DeletionStatus::Failed);
        assert!(failed.existing_deletion.is_none(), "{case}: {failed:?}");
        assert!(!failed.force_eligible);
        assert!(stack
            .store
            .lock()
            .unwrap()
            .state(deletion::GUARDS, "parent")
            .is_null());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_overlap_reviews_blocked_child_then_explicit_force_allows_normal_parent() {
    let stack = Stack::new("running").await;
    stack.store.lock().unwrap().put(
        deletion::DISPATCHES,
        "legacy",
        json!({
            "session_id":"grandchild1", "function_id":"browser::fetch"
        }),
    );
    let accepted = stack.request("child2").await;
    let blocked = stack.run(&accepted.operation_id).await;
    assert!(blocked.force_eligible);
    let parent = stack.request("parent").await;
    assert_eq!(
        parent.failure_code,
        Some(deletion::DeletionFailureCode::OverlappingDeletion)
    );
    assert!(!parent.force_eligible);
    assert_eq!(
        parent.existing_deletion,
        Some(deletion::ExistingDeletion {
            operation_id: blocked.operation_id.clone(),
            session_id: "child2".into(),
        })
    );
    assert!(force(&stack, &parent).await.is_err());
    let recovered = deletion::status(
        &stack.deps,
        deletion::StatusRequest {
            operation_id: blocked.operation_id.clone(),
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(recovered.attempt, blocked.attempt);
    assert!(recovered.force_eligible);
    let authorized = force(&stack, &recovered).await.unwrap();
    assert_eq!(
        stack.run(&authorized.operation_id).await.status,
        DeletionStatus::Completed
    );
    {
        let store = stack.store.lock().unwrap();
        assert!(store.sessions.contains_key("parent"));
        assert!(store.sessions.contains_key("child1"));
        assert_eq!(
            store.state(deletion::GUARDS, "child2"),
            json!(blocked.operation_id)
        );
    }
    stack.set_status("parent", "completed");
    stack.set_status("child1", "completed");
    let retry = stack.request("parent").await;
    assert_eq!(retry.status, DeletionStatus::Deleting, "{retry:?}");
    assert_eq!(
        stack.run(&retry.operation_id).await.status,
        DeletionStatus::Completed
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_overlap_unknown_owner_never_exposes_raw_identity() {
    let stack = Stack::new("completed").await;
    stack.store.lock().unwrap().put(
        deletion::GUARDS,
        "grandchild1",
        json!("secret-foreign-owner"),
    );
    let parent = stack.request("parent").await;
    assert_eq!(parent.status, DeletionStatus::Failed);
    assert_eq!(
        parent.failure_code,
        Some(deletion::DeletionFailureCode::Failed)
    );
    assert!(parent.existing_deletion.is_none());
    assert!(!parent.force_eligible);
    assert!(!serde_json::to_string(&parent)
        .unwrap()
        .contains("secret-foreign-owner"));
}

async fn force(
    stack: &Stack,
    normal: &deletion::Snapshot,
) -> Result<deletion::Snapshot, harness::error::HarnessError> {
    deletion::handle(&stack.deps, serde_json::from_value(json!({
        "session_id":normal.session_id,"mode":"force","operation_id":normal.operation_id,"attempt":normal.attempt
    })).unwrap()).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_legacy_witness_deletes_only_selected_subtree_and_replays_confirmation() {
    let mut stack = Stack::new("running").await;
    {
        let mut store = stack.store.lock().unwrap();
        store.put(
            deletion::DISPATCHES,
            "legacy",
            json!({"session_id":"grandchild1","function_id":"browser::fetch"}),
        );
        store.put(
            deletion::DISPATCHES,
            "unrelated",
            json!({"session_id":"child1","function_id":"slow::work"}),
        );
    }
    let accepted = stack.request("child2").await;
    let normal = stack.run(&accepted.operation_id).await;
    assert!(normal.force_eligible);
    assert_eq!(normal.mode, deletion::DeletionMode::Normal);
    assert_eq!(normal.remaining_session_ids, ["child2", "grandchild1"]);
    let escalated = force(&stack, &normal).await.unwrap();
    assert_eq!(escalated.attempt, normal.attempt + 1);
    assert_eq!(force(&stack, &normal).await.unwrap(), escalated);
    stack.reset_runtime();
    deletion::recover(&stack.deps).await.unwrap();
    let done = stack.run(&escalated.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    assert_eq!(force(&stack, &normal).await.unwrap(), done);
    let store = stack.store.lock().unwrap();
    assert_eq!(
        store.sessions.keys().cloned().collect::<Vec<_>>(),
        ["child1", "parent"]
    );
    assert!(store.state(deletion::DISPATCHES, "legacy").is_null());
    assert_eq!(
        store.state(deletion::DISPATCHES, "unrelated")["session_id"],
        "child1"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_pending_external_call_late_resolve_send_spawn_and_step_do_not_recreate_after_restart(
) {
    let mut stack = Stack::new("completed").await;
    stack.park_on_external_call("grandchild1");
    let accepted = stack.request("child2").await;
    let normal = stack.run(&accepted.operation_id).await;
    assert!(normal.force_eligible);
    let escalated = force(&stack, &normal).await.unwrap();
    assert_eq!(
        stack.run(&escalated.operation_id).await.status,
        DeletionStatus::Completed
    );
    stack.reset_runtime();
    let before = stack.store.lock().unwrap().calls.len();
    let reply = harness::functions::function_resolve::handle(&stack.deps, serde_json::from_value(json!({
        "session_id":"grandchild1","turn_id":"t_grandchild1","function_call_id":"ext-1","content":[]
    })).unwrap()).await.unwrap();
    assert!(!reply.resolved);
    assert!(harness::functions::send::handle(
        &stack.deps,
        serde_json::from_value(json!({"session_id":"grandchild1","message":"late"})).unwrap()
    )
    .await
    .is_err());
    assert!(harness::subagent::spawn_child(&stack.deps, &serde_json::from_value(json!({"session_id":"late","parent_session_id":"grandchild1","task":"late","model":"fake"})).unwrap(), None).await.is_err());
    let reply = harness::turn_loop::run_step(
        &stack.deps,
        serde_json::from_value(
            json!({"session_id":"grandchild1","turn_id":"t_grandchild1","step":0,"depth":0}),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(reply.skipped);
    assert!(
        !harness::session_status::reconcile(&stack.deps, "grandchild1")
            .await
            .unwrap()
    );
    let store = stack.store.lock().unwrap();
    assert!(store.state("harness_turn", "grandchild1").is_null());
    assert!(!store.sessions.contains_key("grandchild1"));
    assert!(!store.calls[before..].iter().any(|(f, _)| [
        "session::append",
        "session::ensure",
        "session::create",
        "session::set-status"
    ]
    .contains(&f.as_str())));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_rejects_absent_stale_wrong_root_changed_topology_and_foreign_guard() {
    let stack = Stack::new("completed").await;
    let request = |root: &str, op: &str, attempt: u32| {
        serde_json::from_value(
            json!({"session_id":root,"mode":"force","operation_id":op,"attempt":attempt}),
        )
        .unwrap()
    };
    assert!(deletion::handle(&stack.deps, request("child2", "none", 1))
        .await
        .is_err());
    stack.park_on_external_call("grandchild1");
    let accepted = stack.request("child2").await;
    let normal = stack.run(&accepted.operation_id).await;
    assert!(
        deletion::handle(&stack.deps, request("child2", &normal.operation_id, 0))
            .await
            .is_err()
    );
    assert!(deletion::handle(
        &stack.deps,
        request("child1", &normal.operation_id, normal.attempt)
    )
    .await
    .is_err());
    stack.session("new-child", Some("child2"), "completed");
    assert!(force(&stack, &normal).await.is_err());
    stack.store.lock().unwrap().sessions.remove("new-child");
    {
        let mut store = stack.store.lock().unwrap();
        let value = store.state(deletion::OPERATIONS, &normal.operation_id);
        let mut corrupt = value.clone();
        corrupt["members"] = json!(["grandchild1", "child2"]);
        store.put(deletion::OPERATIONS, &normal.operation_id, corrupt);
    }
    assert!(force(&stack, &normal).await.is_err());
    {
        let mut store = stack.store.lock().unwrap();
        let mut value = store.state(deletion::OPERATIONS, &normal.operation_id);
        value["members"] = json!(["child2", "grandchild1"]);
        store.put(deletion::OPERATIONS, &normal.operation_id, value);
        store.put(deletion::GUARDS, "grandchild1", Value::Null);
    }
    assert!(force(&stack, &normal).await.is_err());
    stack
        .store
        .lock()
        .unwrap()
        .put(deletion::GUARDS, "grandchild1", json!("foreign"));
    assert!(force(&stack, &normal).await.is_err());
    assert_eq!(stack.store.lock().unwrap().sessions.len(), 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_never_erases_under_local_writer_and_explicit_retry_preserves_scope() {
    let stack = Stack::new("completed").await;
    stack.park_on_external_call("grandchild1");
    let normal = stack.request("child2").await;
    let normal = stack.run(&normal.operation_id).await;
    let held = stack.deps.turn_activity.guard("grandchild1").await;
    let escalated = force(&stack, &normal).await.unwrap();
    let started = std::time::Instant::now();
    let blocked = stack.run(&escalated.operation_id).await;
    assert_eq!(blocked.status, DeletionStatus::Failed);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(blocked.blockers.len(), 1);
    assert_eq!(blocked.blockers[0].session_id, "grandchild1");
    assert_eq!(
        blocked.blockers[0].kind,
        deletion::BlockerKind::ActiveProcessing
    );
    assert_eq!(stack.store.lock().unwrap().sessions.len(), 4);
    drop(held);
    // An ordinary retry cannot silently run force.
    assert_eq!(stack.request("child2").await, blocked);
    let retry = force(&stack, &blocked).await.unwrap();
    assert_eq!(retry.attempt, escalated.attempt + 1);
    assert_eq!(
        stack.run(&retry.operation_id).await.status,
        DeletionStatus::Completed
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_partial_failure_retains_scope_and_retry_recovers_deleted_child() {
    let stack = Stack::new("completed").await;
    stack.park_on_external_call("grandchild1");
    let normal = stack.request("child2").await;
    let normal = stack.run(&normal.operation_id).await;
    stack.store.lock().unwrap().fail_delete = Some("child2".into());
    let escalated = force(&stack, &normal).await.unwrap();
    let partial = stack.run(&escalated.operation_id).await;
    assert_eq!(partial.status, DeletionStatus::Failed);
    assert!(!partial.data_retained);
    assert_eq!(partial.deleted_session_ids, ["grandchild1"]);
    assert_eq!(partial.remaining_session_ids, ["child2"]);
    assert_eq!(partial.unconfirmed_session_ids, ["child2"]);
    assert!(!partial.error.as_ref().unwrap().contains("data retained"));
    stack.store.lock().unwrap().fail_delete = None;
    let retry = force(&stack, &partial).await.unwrap();
    assert_eq!(
        stack.run(&retry.operation_id).await.status,
        DeletionStatus::Completed
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolved_blockers_refresh_eligibility_and_retry_uses_normal_deletion() {
    let stack = Stack::new("completed").await;
    stack.park_on_external_call("grandchild1");
    let normal = stack.request("child2").await;
    let normal = stack.run(&normal.operation_id).await;
    assert!(normal.force_eligible);
    {
        let mut store = stack.store.lock().unwrap();
        let mut record = store.state("harness_turn", "grandchild1");
        record["status"] = json!("completed");
        record["calls"] = json!({});
        store.put("harness_turn", "grandchild1", record);
        store
            .state
            .retain(|(scope, _), _| scope != deletion::DISPATCHES);
    }
    let refreshed = deletion::status(
        &stack.deps,
        StatusRequest {
            operation_id: normal.operation_id.clone(),
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!refreshed.force_eligible);
    assert!(refreshed.blockers.is_empty());
    assert!(force(&stack, &normal).await.is_err());
    let retry = stack.request("child2").await;
    assert_eq!(retry.mode, deletion::DeletionMode::Normal);
    assert_eq!(
        stack.run(&retry.operation_id).await.status,
        DeletionStatus::Completed
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_lost_delete_ack_recovers_after_restart_without_expanding_scope() {
    for missing in ["grandchild1", "child2"] {
        let mut stack = Stack::new("completed").await;
        stack.park_on_external_call("grandchild1");
        let normal = stack.request("child2").await;
        let normal = stack.run(&normal.operation_id).await;
        let accepted = force(&stack, &normal).await.unwrap();
        if missing == "child2" {
            stack.store.lock().unwrap().fail_delete = Some("child2".into());
            let partial = stack.run(&accepted.operation_id).await;
            assert_eq!(partial.deleted_session_ids, ["grandchild1"]);
            stack.store.lock().unwrap().fail_delete = None;
            force(&stack, &partial).await.unwrap();
        }
        stack.store.lock().unwrap().fail_after_delete_once = true;
        let failed = stack.run(&accepted.operation_id).await;
        assert_eq!(failed.status, DeletionStatus::Failed);
        assert!(!stack.store.lock().unwrap().sessions.contains_key(missing));
        assert_eq!(failed.unconfirmed_session_ids, [missing]);
        assert!(failed.remaining_session_ids.contains(&missing.into()));
        assert!(!failed.deleted_session_ids.contains(&missing.into()));
        assert!(!failed.error.as_ref().unwrap().contains("data retained"));
        if missing == "grandchild1" {
            assert!(failed.deleted_session_ids.is_empty());
            assert_eq!(failed.remaining_session_ids, ["child2", "grandchild1"]);
        } else {
            assert_eq!(failed.deleted_session_ids, ["grandchild1"]);
            assert_eq!(failed.remaining_session_ids, ["child2"]);
        }
        assert_eq!(
            deletion::status(
                &stack.deps,
                StatusRequest {
                    operation_id: failed.operation_id.clone(),
                }
            )
            .await
            .unwrap()
            .unwrap(),
            failed
        );
        // Old persisted checkpoints lack the additive wire field, but the
        // durable intent still makes their intermediate outcome unconfirmed.
        {
            let mut store = stack.store.lock().unwrap();
            let mut op = store.state(deletion::OPERATIONS, &failed.operation_id);
            op["snapshot"]
                .as_object_mut()
                .unwrap()
                .remove("unconfirmed_session_ids");
            store.put(deletion::OPERATIONS, &failed.operation_id, op);
        }
        stack.reset_runtime();
        assert_eq!(
            deletion::status(
                &stack.deps,
                StatusRequest {
                    operation_id: failed.operation_id.clone(),
                }
            )
            .await
            .unwrap()
            .unwrap()
            .unconfirmed_session_ids,
            [missing]
        );
        let retry = force(&stack, &failed).await.unwrap();
        let done = stack.run(&retry.operation_id).await;
        assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
        assert_eq!(done.remaining_session_ids, Vec::<String>::new());
        assert!(done.unconfirmed_session_ids.is_empty());
        assert_eq!(
            stack
                .store
                .lock()
                .unwrap()
                .sessions
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["child1", "parent"]
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_force_confirmations_increment_once_and_cleanup_only_once() {
    let stack = Stack::new("completed").await;
    stack.park_on_external_call("grandchild1");
    let normal = stack.request("child2").await;
    let normal = stack.run(&normal.operation_id).await;
    let (one, two) = tokio::join!(force(&stack, &normal), force(&stack, &normal));
    let one = one.unwrap();
    assert_eq!(one, two.unwrap());
    assert_eq!(one.attempt, normal.attempt + 1);
    let (one, two) = tokio::join!(stack.run(&one.operation_id), stack.run(&one.operation_id));
    assert_eq!(one, two);
    assert_eq!(one.status, DeletionStatus::Completed);
    let store = stack.store.lock().unwrap();
    let deleted: Vec<_> = store
        .calls
        .iter()
        .filter(|(f, _)| f == "session::delete")
        .map(|(_, p)| p["session_id"].as_str().unwrap())
        .collect();
    assert_eq!(deleted, ["grandchild1", "child2"]);
}

#[test]
fn fixture_rejects_public_reserved_listing_and_foreign_private_scope() {
    let mut store = Store::default();
    store.private_scopes.insert(deletion::DISPATCHES.into());
    store.put(
        deletion::DISPATCHES,
        "secret-key",
        json!({"session_id":"s"}),
    );
    for function in [
        "state::get",
        "state::list",
        "state::list_keys",
        "state::set",
        "state::delete",
    ] {
        assert!(store
            .respond(
                function,
                &json!({"scope":deletion::DISPATCHES,"key":"secret-key"}),
                &Value::Null
            )
            .unwrap_err()
            .contains("RESERVED_SCOPE"));
    }
    assert!(store
        .respond(
            "harness::state::list_keys",
            &json!({"scope":"foreign-private"}),
            &Value::Null
        )
        .unwrap_err()
        .contains("INVALID_SCOPE"));
    assert_eq!(
        store
            .respond(
                "harness::state::list_keys",
                &json!({"scope":deletion::DISPATCHES}),
                &Value::Null
            )
            .unwrap(),
        json!({"keys":["secret-key"]})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_key_listing_failure_retains_data_and_never_falls_back_to_public() {
    let stack = Stack::new("running").await;
    stack.park_on_external_call("child2");
    let accepted = stack.request("child2").await;
    let normal = stack.run(&accepted.operation_id).await;
    let force = force(&stack, &normal).await.unwrap();
    stack
        .store
        .lock()
        .unwrap()
        .fail
        .insert("harness::state::list_entries".into());
    let failed = stack.run(&force.operation_id).await;
    assert_eq!(failed.status, DeletionStatus::Failed);
    assert!(failed.deleted_session_ids.is_empty());
    let store = stack.store.lock().unwrap();
    assert!(store.sessions.contains_key("child2"));
    assert!(store.sessions.contains_key("grandchild1"));
    assert!(!store
        .calls
        .iter()
        .any(|(f, d)| f == "state::list_entries" && d["scope"] == deletion::DISPATCHES));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_private_key_accessor_recovers_namespace_without_public_fallback() {
    let mut stack = Stack::new("running").await;
    stack.store.lock().unwrap().put(
        deletion::DISPATCHES,
        "no-embedded-key",
        json!({"session_id":"child2","function_id":"slow::legacy"}),
    );
    let accepted = stack.request("child2").await;
    let normal = stack.run(&accepted.operation_id).await;
    let escalated = force(&stack, &normal).await.unwrap();
    stack.reset_runtime();
    {
        let mut store = stack.store.lock().unwrap();
        store.private_scopes.clear();
        store.private_accessors_missing = true;
        store.calls.clear();
    }
    deletion::recover(&stack.deps).await.unwrap();
    let done = stack.run(&escalated.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    let store = stack.store.lock().unwrap();
    assert!(store
        .state(deletion::DISPATCHES, "no-embedded-key")
        .is_null());
    assert!(store
        .calls
        .iter()
        .any(|(f, _)| f == "state::claim-namespace"));
    assert!(store
        .calls
        .iter()
        .any(|(f, d)| f == "harness::state::list_entries" && d["scope"] == deletion::DISPATCHES));
    assert!(!store
        .calls
        .iter()
        .any(|(f, d)| f == "state::list_entries" && d["scope"] == deletion::DISPATCHES));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_purge_cas_preserves_a_replaced_foreign_witness_and_retains_data() {
    let stack = Stack::new("running").await;
    stack.store.lock().unwrap().put(
        deletion::DISPATCHES,
        "replace-me",
        json!({"session_id":"grandchild1","function_id":"slow::legacy"}),
    );
    let accepted = stack.request("child2").await;
    let normal = stack.run(&accepted.operation_id).await;
    let escalated = force(&stack, &normal).await.unwrap();
    let foreign = json!({"session_id":"child1","function_id":"other::new"});
    stack.store.lock().unwrap().mutate_dispatch_on_read = Some(foreign.clone());
    let failed = stack.run(&escalated.operation_id).await;
    assert_eq!(failed.status, DeletionStatus::Failed);
    assert!(failed.deleted_session_ids.is_empty());
    let store = stack.store.lock().unwrap();
    assert_eq!(store.state(deletion::DISPATCHES, "replace-me"), foreign);
    assert!(store.sessions.contains_key("grandchild1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_erase_refuses_active_witness_admission_then_fences_queued_dispatch() {
    let stack = Stack::new("running").await;
    stack.park_on_external_call("child2");
    let accepted = stack.request("child2").await;
    let normal = stack.run(&accepted.operation_id).await;
    let escalated = force(&stack, &normal).await.unwrap();
    let barrier = stack.deps.dispatch_admission.guard("grandchild1").await;
    let engine = stack.deps.engine().await;
    let policy = harness::policy::CompiledPolicy::from(None);
    let args = json!({});
    let dispatch = harness::functions::subscribe::invoke(
        &stack.deps,
        &engine,
        &policy,
        "ext::must-not-run",
        &args,
        "grandchild1",
        false,
        None,
    );
    tokio::pin!(dispatch);
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut dispatch)
            .await
            .is_err()
    );
    let blocked = stack.run(&escalated.operation_id).await;
    assert_eq!(blocked.status, DeletionStatus::Failed);
    assert!(blocked.deleted_session_ids.is_empty());
    drop(barrier);
    assert!(dispatch.await.is_error);
    let retry = force(&stack, &blocked).await.unwrap();
    assert_eq!(
        stack.run(&retry.operation_id).await.status,
        DeletionStatus::Completed
    );
    let store = stack.store.lock().unwrap();
    assert!(dispatch_witnesses(&store).is_empty());
    assert!(!store.calls.iter().any(|(f, _)| f == "ext::must-not-run"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admission_before_guard_with_failed_withdrawal_cannot_land_behind_force_purge() {
    let stack = Stack::new("completed").await;
    let gate = Arc::new(Notify::new());
    {
        let mut store = stack.store.lock().unwrap();
        store.hold_dispatch_reply = Some(gate.clone());
        store.fail_dispatch_clear = true;
    }
    let deps = stack.deps.clone();
    let dispatch = tokio::spawn(async move {
        let engine = deps.engine().await;
        harness::functions::subscribe::invoke(
            &deps,
            &engine,
            &harness::policy::CompiledPolicy::from(None),
            "ext::never-started",
            &json!({}),
            "grandchild1",
            false,
            None,
        )
        .await
    });
    stack
        .wait_for(|store| dispatch_witnesses(store).len() == 1)
        .await;
    // The witness CAS was applied but its acknowledgement is held. Guard
    // claiming and normal/force processing can proceed on the same transport.
    let accepted = stack.request("child2").await;
    let normal = stack.run(&accepted.operation_id).await;
    assert!(normal.force_eligible);
    let escalated = force(&stack, &normal).await.unwrap();
    let blocked = stack.run(&escalated.operation_id).await;
    assert_eq!(blocked.status, DeletionStatus::Failed);
    assert!(blocked.deleted_session_ids.is_empty());
    gate.notify_one();
    assert!(dispatch.await.unwrap().is_error);
    {
        let mut store = stack.store.lock().unwrap();
        assert_eq!(
            dispatch_witnesses(&store).len(),
            1,
            "failed withdrawal remains visible"
        );
        assert!(!store.calls.iter().any(|(f, _)| f == "ext::never-started"));
        store.fail_dispatch_clear = false;
    }
    let retry = force(&stack, &blocked).await.unwrap();
    assert_eq!(
        stack.run(&retry.operation_id).await.status,
        DeletionStatus::Completed
    );
    assert!(dispatch_witnesses(&stack.store.lock().unwrap()).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confirmed_reply_after_force_purge_cas_accepts_null_without_recreation() {
    let stack = Stack::new("running").await;
    let gate = Arc::new(Notify::new());
    stack.store.lock().unwrap().hold_target_reply =
        Some(("ext::late-confirmed".into(), gate.clone()));
    let deps = stack.deps.clone();
    let dispatch = tokio::spawn(async move {
        let engine = deps.engine().await;
        harness::functions::subscribe::invoke(
            &deps,
            &engine,
            &harness::policy::CompiledPolicy::from(None),
            "ext::late-confirmed",
            &json!({}),
            "grandchild1",
            false,
            None,
        )
        .await
    });
    stack
        .wait_for(|store| store.calls.iter().any(|(f, _)| f == "ext::late-confirmed"))
        .await;
    let accepted = stack.request("child2").await;
    // Healthy local dispatches now receive normal grace. Shorten only this
    // fixture's operation deadline, so Force precedes the SDK's 2 s timeout
    // and the late CONFIRMED response still exercises end_dispatch's CAS.
    {
        let mut store = stack.store.lock().unwrap();
        let mut row = store.state(deletion::OPERATIONS, &accepted.operation_id);
        row["deadline"] = json!(harness::types::message::AgentMessage::now_ms() + 300);
        store.put(deletion::OPERATIONS, &accepted.operation_id, row);
    }
    let normal = stack.run(&accepted.operation_id).await;
    let escalated = force(&stack, &normal).await.unwrap();
    let done = stack.run(&escalated.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    let cas_before = witness_rpcs(&stack.store.lock().unwrap()).len();
    gate.notify_one();
    assert!(dispatch.await.unwrap().is_error); // Confirmed target error, not a timeout.
    let store = stack.store.lock().unwrap();
    assert_eq!(
        witness_rpcs(&store).len(),
        cas_before + 1,
        "late end_dispatch performed one CAS"
    );
    assert!(dispatch_witnesses(&store).is_empty());
    assert!(store.state("harness_turn", "grandchild1").is_null());
    assert!(!store.sessions.contains_key("grandchild1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn applied_delete_with_failed_existence_read_stays_unconfirmed_until_explicit_retry() {
    let mut stack = Stack::new("completed").await;
    stack.park_on_external_call("grandchild1");
    let normal = stack.request("child2").await;
    let normal = stack.run(&normal.operation_id).await;
    let accepted = force(&stack, &normal).await.unwrap();
    stack.store.lock().unwrap().fail_read_after_delete = true;
    let failed = stack.run(&accepted.operation_id).await;
    assert_eq!(failed.status, DeletionStatus::Failed);
    assert!(failed.deleted_session_ids.is_empty());
    assert_eq!(failed.remaining_session_ids, ["child2", "grandchild1"]);
    assert_eq!(failed.unconfirmed_session_ids, ["grandchild1"]);
    assert!(!failed.error.as_ref().unwrap().contains("data retained"));
    assert!(!stack
        .store
        .lock()
        .unwrap()
        .sessions
        .contains_key("grandchild1"));
    stack.reset_runtime();
    let caught_up = deletion::status(
        &stack.deps,
        StatusRequest {
            operation_id: failed.operation_id.clone(),
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(caught_up, failed);
    assert!(
        force(&stack, &caught_up).await.is_err(),
        "failed read must not authorize retry"
    );
    {
        let mut store = stack.store.lock().unwrap();
        store.fail_read_after_delete = false;
        store.fail_session_read = None;
    }
    let retry = force(&stack, &caught_up).await.unwrap();
    let done = stack.run(&retry.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed);
    assert!(done.unconfirmed_session_ids.is_empty());
    assert!(done.remaining_session_ids.is_empty());
}

// Independent review reproductions promoted to regression tests.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_r1_legacy_planned_operation_without_links_can_be_forced() {
    let stack = Stack::new("completed").await;
    stack.park_on_external_call("grandchild1");
    let accepted = stack.request("child2").await;
    let started = std::time::Instant::now();
    let first = stack.run(&accepted.operation_id).await;
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(first.blockers.len(), 1);
    assert_eq!(first.blockers[0].call_id.as_deref(), Some("ext-1"));
    assert!(first.force_eligible, "{first:?}");
    assert!(first.data_retained);
    // Rewrite the persisted row into the pre-feature (HEAD) storage shape:
    // planned, members and parent persisted, but no links/new snapshot fields.
    {
        let mut store = stack.store.lock().unwrap();
        let mut op = store.state(deletion::OPERATIONS, &first.operation_id);
        let obj = op.as_object_mut().unwrap();
        for k in [
            "links",
            "force_confirmed_attempt",
            "erasing",
            "cleanup_started",
        ] {
            obj.remove(k);
        }
        let snap = obj.get_mut("snapshot").unwrap().as_object_mut().unwrap();
        for k in [
            "remaining_session_ids",
            "unconfirmed_session_ids",
            "mode",
            "blockers",
            "force_eligible",
            "failure_code",
            "data_retained",
        ] {
            snap.remove(k);
        }
        snap.insert(
            "error".into(),
            json!("deletion deadline expired; unconfirmed sessions and data retained"),
        );
        println!("R1 legacy row: {op}");
        store.put(deletion::OPERATIONS, &first.operation_id, op);
    }
    let retry = stack.request("child2").await;
    let blocked = stack.run(&retry.operation_id).await;
    println!("R1 legacy normal retry: {blocked:?}");
    assert!(blocked.force_eligible, "{blocked:?}");
    let forced = force(&stack, &blocked).await;
    println!("R1 legacy force confirmation: {forced:?}");
    let forced = forced.expect("a pre-feature planned operation must be force-confirmable");
    let done = stack.run(&forced.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .sessions
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["child1", "parent"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_r2_normal_delete_of_running_turn_survives_slow_cancellation() {
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
    // Healthy cancellation that needs 1.5 s to finalize (well under the 120 s deadline).
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    stack.set_status("grandchild1", "cancelled");
    let done = tokio::time::timeout(Duration::from_secs(5), running)
        .await
        .unwrap()
        .unwrap();
    println!("R2 slow-cancellation normal result: {done:?}");
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_r3_force_purge_cost_scales_with_historical_dispatch_keys() {
    let stack = Stack::new("completed").await;
    // Ordinary confirmed dispatches on an unrelated live session. Each leaves
    // a permanent null-valued key (CAS to null keeps the key, as in KvStore/Redis).
    let engine = stack.deps.engine().await;
    let policy = harness::policy::CompiledPolicy::from(None);
    for _ in 0..150 {
        let _ = harness::functions::subscribe::invoke(
            &stack.deps,
            &engine,
            &policy,
            "ext::historic",
            &json!({}),
            "parent",
            false,
            None,
        )
        .await;
    }
    let historic = stack
        .store
        .lock()
        .unwrap()
        .state
        .keys()
        .filter(|(s, _)| s == deletion::DISPATCHES)
        .count();
    assert!(historic >= 150);
    stack.park_on_external_call("grandchild1");
    {
        let mut store = stack.store.lock().unwrap();
        for (key, sid) in [
            ("target-a", "child2"),
            ("target-b", "grandchild1"),
            ("foreign", "parent"),
        ] {
            store.put(
                deletion::DISPATCHES,
                key,
                json!({"session_id":sid,"function_id":"slow::target"}),
            );
        }
    }
    let accepted = stack.request("child2").await;
    let normal = stack.run(&accepted.operation_id).await;
    let escalated = force(&stack, &normal).await.unwrap();
    let before = stack.store.lock().unwrap().calls.len();
    let done = stack.run(&escalated.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    let store = stack.store.lock().unwrap();
    let gets = store.calls[before..]
        .iter()
        .filter(|(f, d)| f == "harness::state::get" && d["scope"] == deletion::DISPATCHES)
        .count();
    println!("R3 historical dispatch keys={historic}; members=2; per-key dispatch reads during one force run={gets}");
    assert_eq!(gets, 0);
    let cas: Vec<_> = store.calls[before..]
        .iter()
        .filter(|(f, d)| {
            f == "harness::state::compare-and-set" && d["scope"] == deletion::DISPATCHES
        })
        .map(|(_, d)| d["key"].as_str().unwrap())
        .collect();
    assert_eq!(cas, ["target-b", "target-a"]);
    assert_eq!(
        store.state(deletion::DISPATCHES, "foreign")["session_id"],
        "parent"
    );
    assert_eq!(
        store.calls[before..]
            .iter()
            .filter(
                |(f, d)| f == "harness::state::list_entries" && d["scope"] == deletion::DISPATCHES
            )
            .count(),
        1 // Only three non-null witnesses remain; retired keys are not paged.
    );
    assert_eq!(
        store.sessions.keys().cloned().collect::<Vec<_>>(),
        ["child1", "parent"]
    );
}

fn strip_legacy_links(stack: &Stack, id: &str) {
    let mut store = stack.store.lock().unwrap();
    let mut row = store.state(deletion::OPERATIONS, id);
    row.as_object_mut().unwrap().remove("links");
    store.put(deletion::OPERATIONS, id, row);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_missing_links_never_expand_reparent_or_accept_explicit_empty_links() {
    for variant in ["expand", "parent", "empty", "order", "foreign"] {
        let stack = Stack::new("completed").await;
        stack.park_on_external_call("grandchild1");
        let accepted = stack.request("child2").await;
        let normal = stack.run(&accepted.operation_id).await;
        strip_legacy_links(&stack, &normal.operation_id);
        match variant {
            "expand" => stack.session("new", Some("grandchild1"), "completed"),
            "parent" => {
                stack
                    .store
                    .lock()
                    .unwrap()
                    .sessions
                    .get_mut("child2")
                    .unwrap()["metadata"]["parent_session_id"] = json!("child1");
            }
            "foreign" => {
                stack
                    .store
                    .lock()
                    .unwrap()
                    .put(deletion::GUARDS, "grandchild1", json!("foreign"))
            }
            _ => {
                let mut store = stack.store.lock().unwrap();
                let mut row = store.state(deletion::OPERATIONS, &normal.operation_id);
                if variant == "empty" {
                    row["links"] = json!([]);
                } else {
                    row["members"] = json!(["grandchild1", "child2"]);
                }
                store.put(deletion::OPERATIONS, &normal.operation_id, row);
            }
        }
        assert!(force(&stack, &normal).await.is_err(), "{variant}");
        let store = stack.store.lock().unwrap();
        assert!(store.sessions.contains_key("child2"));
        assert!(store.sessions.contains_key("grandchild1"));
        assert!(!store.calls.iter().any(|(f, _)| f == "session::delete"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_missing_links_migrate_only_justified_deleted_or_absent_intent_members() {
    for intent in [false, true] {
        let stack = Stack::new("completed").await;
        stack.park_on_external_call("grandchild1");
        let accepted = stack.request("child2").await;
        let normal = stack.run(&accepted.operation_id).await;
        let escalated = force(&stack, &normal).await.unwrap();
        {
            let mut store = stack.store.lock().unwrap();
            store.sessions.remove("grandchild1");
            let mut row = store.state(deletion::OPERATIONS, &normal.operation_id);
            row.as_object_mut().unwrap().remove("links");
            if intent {
                row["erasing"] = json!("grandchild1");
                row["snapshot"]["unconfirmed_session_ids"] = json!(["grandchild1"]);
            } else {
                row["snapshot"]["deleted_session_ids"] = json!(["grandchild1"]);
                row["snapshot"]["remaining_session_ids"] = json!(["child2"]);
            }
            store.put(deletion::OPERATIONS, &normal.operation_id, row);
        }
        let done = stack.run(&escalated.operation_id).await;
        assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
        assert_eq!(done.deleted_session_ids, ["grandchild1", "child2"]);
        assert_eq!(stack.store.lock().unwrap().sessions.len(), 2);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_dispatch_snapshot_ignores_proven_foreign_malformed_values_only() {
    for (row, succeeds) in [
        (json!({"session_id":"parent","function_id":42}), true),
        (json!({"session_id":"grandchild1","function_id":42}), false),
        (json!({"function_id":"unknown::owner"}), false),
        (json!({"session_id":42}), false),
    ] {
        let stack = Stack::new("completed").await;
        stack.park_on_external_call("grandchild1");
        let accepted = stack.request("child2").await;
        let normal = stack.run(&accepted.operation_id).await;
        let escalated = force(&stack, &normal).await.unwrap();
        stack
            .store
            .lock()
            .unwrap()
            .put(deletion::DISPATCHES, "malformed", row.clone());
        let result = stack.run(&escalated.operation_id).await;
        assert_eq!(
            result.status == DeletionStatus::Completed,
            succeeds,
            "{result:?}"
        );
        let store = stack.store.lock().unwrap();
        assert_eq!(store.state(deletion::DISPATCHES, "malformed"), row);
        assert_eq!(store.sessions.len(), if succeeds { 2 } else { 4 });
        assert!(!store
            .calls
            .iter()
            .any(|(f, d)| f == "harness::state::compare-and-set" && d["key"] == "malformed"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_snapshot_and_target_cas_both_respect_durable_deadline() {
    for function in [
        "harness::state::list_entries",
        "harness::state::compare-and-set",
    ] {
        let stack = Stack::new("completed").await;
        stack.park_on_external_call("grandchild1");
        stack.store.lock().unwrap().put(
            deletion::DISPATCHES,
            "target",
            json!({"session_id":"grandchild1","function_id":"slow::target"}),
        );
        let accepted = stack.request("child2").await;
        let normal = stack.run(&accepted.operation_id).await;
        let escalated = force(&stack, &normal).await.unwrap();
        let gate = Arc::new(Notify::new());
        {
            let mut store = stack.store.lock().unwrap();
            let mut row = store.state(deletion::OPERATIONS, &normal.operation_id);
            row["deadline"] = json!(harness::types::message::AgentMessage::now_ms() + 350);
            store.put(deletion::OPERATIONS, &normal.operation_id, row);
            store.hold_target_reply = Some((function.into(), gate.clone()));
        }
        let started = std::time::Instant::now();
        let failed = stack.run(&escalated.operation_id).await;
        gate.notify_one();
        assert_eq!(failed.status, DeletionStatus::Failed);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(failed.deleted_session_ids.is_empty());
        assert!(!failed.force_eligible);
        assert_eq!(stack.store.lock().unwrap().sessions.len(), 4);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn normal_waits_for_healthy_abort_rpc_and_classifies_only_a_stuck_member() {
    let stack = Stack::new("completed").await;
    stack.set_status("grandchild1", "running");
    {
        let mut store = stack.store.lock().unwrap();
        let mut turn = store.state("harness_turn", "grandchild1");
        turn["stream_request_id"] = json!("healthy-abort");
        store.put("harness_turn", "grandchild1", turn);
    }
    let accepted = stack.request("child2").await;
    let gate = Arc::new(Notify::new());
    stack.store.lock().unwrap().hold_target_reply = Some(("router::abort".into(), gate.clone()));
    let deps = stack.deps.clone();
    let id = accepted.operation_id.clone();
    let job = tokio::spawn(async move {
        deletion::run(&deps, StatusRequest { operation_id: id })
            .await
            .unwrap()
            .unwrap()
    });
    stack
        .wait_for(|s| s.calls.iter().any(|(f, _)| f == "router::abort"))
        .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    stack.set_status("grandchild1", "cancelled");
    gate.notify_one();
    assert_eq!(job.await.unwrap().status, DeletionStatus::Completed);

    let stack = Stack::new("completed").await;
    stack.set_status("grandchild1", "running");
    let accepted = stack.request("child2").await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut row = store.state(deletion::OPERATIONS, &accepted.operation_id);
        row["deadline"] = json!(harness::types::message::AgentMessage::now_ms() + 350);
        store.put(deletion::OPERATIONS, &accepted.operation_id, row);
    }
    let started = std::time::Instant::now();
    let failed = stack.run(&accepted.operation_id).await;
    assert_eq!(failed.status, DeletionStatus::Failed);
    assert!(failed.data_retained);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(failed.blockers.len(), 1);
    assert_eq!(
        failed.blockers[0].kind,
        deletion::BlockerKind::ActiveProcessing
    );
    assert_eq!(failed.blockers[0].session_id, "grandchild1");
    assert!(failed.force_eligible);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_grants_cancelled_local_writer_a_short_grace_on_first_attempt() {
    let stack = Stack::new("completed").await;
    stack.park_on_external_call("grandchild1");
    let accepted = stack.request("child2").await;
    let normal = stack.run(&accepted.operation_id).await;
    let held = stack.deps.turn_activity.guard("grandchild1").await;
    let escalated = force(&stack, &normal).await.unwrap();
    let deps = stack.deps.clone();
    let id = escalated.operation_id.clone();
    let job = tokio::spawn(async move {
        deletion::run(&deps, StatusRequest { operation_id: id })
            .await
            .unwrap()
            .unwrap()
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(stack.store.lock().unwrap().sessions.len(), 4);
    drop(held);
    assert_eq!(job.await.unwrap().status, DeletionStatus::Completed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_reads_persisted_progress_while_erase_is_in_flight() {
    let stack = Stack::new("completed").await;
    let accepted = stack.request("child2").await;
    let gate = Arc::new(Notify::new());
    stack.store.lock().unwrap().hold_target_reply = Some(("session::delete".into(), gate.clone()));
    let deps = stack.deps.clone();
    let id = accepted.operation_id.clone();
    let job = tokio::spawn(async move {
        deletion::run(&deps, StatusRequest { operation_id: id })
            .await
            .unwrap()
            .unwrap()
    });
    stack
        .wait_for(|s| s.calls.iter().any(|(f, _)| f == "session::delete"))
        .await;
    let read = tokio::time::timeout(
        Duration::from_millis(300),
        deletion::status(
            &stack.deps,
            StatusRequest {
                operation_id: accepted.operation_id.clone(),
            },
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert_eq!(read.status, DeletionStatus::Deleting);
    assert_eq!(read.unconfirmed_session_ids, ["grandchild1"]);
    assert!(!read.data_retained);
    gate.notify_one();
    assert_eq!(job.await.unwrap().status, DeletionStatus::Completed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_session_after_lost_delete_ack_is_an_idempotent_success() {
    let stack = Stack::new("completed").await;
    let first = stack.request("child2").await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut op = store.state(deletion::OPERATIONS, &first.operation_id);
        let obj = op.as_object_mut().unwrap();
        for k in [
            "links",
            "force_confirmed_attempt",
            "erasing",
            "cleanup_started",
        ] {
            obj.remove(k);
        }
        obj.insert("planned".into(), json!(true));
        obj.insert("notified".into(), json!(true));
        obj.insert("members".into(), json!(["child2", "grandchild1"]));
        obj.insert("parent".into(), json!("parent"));
        let snap = obj.get_mut("snapshot").unwrap().as_object_mut().unwrap();
        for k in [
            "remaining_session_ids",
            "unconfirmed_session_ids",
            "mode",
            "blockers",
            "force_eligible",
            "failure_code",
            "data_retained",
        ] {
            snap.remove(k);
        }
        snap.insert("status".into(), json!("failed"));
        snap.insert(
            "error".into(),
            json!("session::delete grandchild1: connection reset"),
        );
        println!("R2A legacy row: {op}");
        store.put(deletion::OPERATIONS, &first.operation_id, op);
        store.put(deletion::GUARDS, "grandchild1", json!(first.operation_id));
        // Pre-deploy erase applied session::delete(grandchild1) but lost its ack.
        store.sessions.remove("grandchild1");
        store.messages.remove("grandchild1");
    }
    let retry = stack.request("child2").await;
    let done = stack.run(&retry.operation_id).await;
    println!("R2A legacy lost-ack normal retry: {done:?}");
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    let store = stack.store.lock().unwrap();
    assert_eq!(
        store.sessions.keys().cloned().collect::<Vec<_>>(),
        ["child1", "parent"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review2_c_stop_and_delete_waits_for_live_step_with_triggered_call() {
    let stack = Stack::new("completed").await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut turn = store.state("harness_turn", "grandchild1");
        turn["status"] = json!("running");
        turn["calls"] = json!({"c1": {"state": "triggered", "function_id": "slow::tool"}});
        store.put("harness_turn", "grandchild1", turn);
    }
    // A live local step is executing c1 (holds the step activity barrier).
    let held = stack.deps.turn_activity.guard("grandchild1").await;
    let accepted = stack.request("child2").await;
    let deps = stack.deps.clone();
    let id = accepted.operation_id.clone();
    let job = tokio::spawn(async move {
        deletion::run(&deps, StatusRequest { operation_id: id })
            .await
            .unwrap()
            .unwrap()
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    let finished_before_step = job.is_finished();
    // The step observes the cancel: c1 settles as stopped and the turn finalizes.
    {
        let mut store = stack.store.lock().unwrap();
        let mut turn = store.state("harness_turn", "grandchild1");
        turn["status"] = json!("cancelled");
        turn["calls"] = json!({"c1": {"state": "done", "function_id": "slow::tool"}});
        store.put("harness_turn", "grandchild1", turn);
    }
    drop(held);
    stack.deps.deletion_changed.notify_waiters();
    let done = tokio::time::timeout(Duration::from_secs(5), job)
        .await
        .unwrap()
        .unwrap();
    println!("R2C live step with triggered call: finished_before_step={finished_before_step} result={done:?}");
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    let store = stack.store.lock().unwrap();
    assert_eq!(
        store.sessions.keys().cloned().collect::<Vec<_>>(),
        ["child1", "parent"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review2_a2_legacy_planned_row_with_unclaimed_descendant_guard_recovers() {
    let stack = Stack::new("completed").await;
    let first = stack.request("child2").await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut op = store.state(deletion::OPERATIONS, &first.operation_id);
        let obj = op.as_object_mut().unwrap();
        for k in [
            "links",
            "force_confirmed_attempt",
            "erasing",
            "cleanup_started",
        ] {
            obj.remove(k);
        }
        obj.insert("planned".into(), json!(true));
        obj.insert("members".into(), json!(["child2", "grandchild1"]));
        obj.insert("parent".into(), json!("parent"));
        let snap = obj.get_mut("snapshot").unwrap().as_object_mut().unwrap();
        for k in [
            "remaining_session_ids",
            "unconfirmed_session_ids",
            "mode",
            "blockers",
            "force_eligible",
            "failure_code",
            "data_retained",
        ] {
            snap.remove(k);
        }
        snap.insert("status".into(), json!("failed"));
        snap.insert("error".into(), json!("state::compare-and-set: timed out"));
        store.put(deletion::OPERATIONS, &first.operation_id, op);
        // Pre-deploy prepare saved the plan, then lost the descendant guard CAS.
        assert!(store.state(deletion::GUARDS, "grandchild1").is_null());
    }
    let retry = stack.request("child2").await;
    let done = stack.run(&retry.operation_id).await;
    println!("R2A2 legacy unclaimed descendant guard: {done:?}");
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    let store = stack.store.lock().unwrap();
    assert_eq!(
        store.sessions.keys().cloned().collect::<Vec<_>>(),
        ["child1", "parent"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review2_b2_real_in_flight_dispatch_waits_for_healthy_reply() {
    let stack = Stack::new("completed").await;
    let gate = Arc::new(Notify::new());
    stack.store.lock().unwrap().hold_target_reply = Some(("ext::healthy".into(), gate.clone()));
    let deps = stack.deps.clone();
    let dispatch = tokio::spawn(async move {
        let engine = deps.engine().await;
        harness::functions::subscribe::invoke(
            &deps,
            &engine,
            &harness::policy::CompiledPolicy::from(None),
            "ext::healthy",
            &json!({}),
            "grandchild1",
            false,
            None,
        )
        .await
    });
    stack
        .wait_for(|s| s.calls.iter().any(|(f, _)| f == "ext::healthy"))
        .await;
    let accepted = stack.request("child2").await;
    let started = std::time::Instant::now();
    let deps = stack.deps.clone();
    let id = accepted.operation_id.clone();
    let deletion = tokio::spawn(async move {
        deletion::run(&deps, StatusRequest { operation_id: id })
            .await
            .unwrap()
            .unwrap()
    });
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert!(
        !deletion.is_finished(),
        "normal must wait for the live invocation"
    );
    gate.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(5), deletion)
        .await
        .unwrap()
        .unwrap();
    let elapsed = started.elapsed();
    let reply = dispatch.await.unwrap();
    println!(
        "R2B2 real in-flight dispatch: elapsed={elapsed:?} result={result:?} reply_is_error={}",
        reply.is_error
    );
    assert_eq!(result.status, DeletionStatus::Completed, "{result:?}");
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .sessions
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["child1", "parent"]
    );
}

/// Preserve HEAD provenance, not a new row rewritten with explicit intent.
fn legacy_head_plan(stack: &Stack, id: &str) {
    let mut store = stack.store.lock().unwrap();
    let mut row = store.state(deletion::OPERATIONS, id);
    let obj = row.as_object_mut().unwrap();
    for key in ["links", "erasing", "cleanup_started"] {
        obj.remove(key);
    }
    row["planned"] = json!(true);
    row["members"] = json!(["child2", "grandchild1"]);
    row["parent"] = json!("parent");
    store.put(deletion::OPERATIONS, id, row);
    store.put(deletion::GUARDS, "grandchild1", json!(id));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review2_legacy_cursor_and_reservations_never_bypass_integrity() {
    for variant in [
        "non-next",
        "foreign",
        "expand",
        "reparent",
        "order",
        "explicit-null",
        "explicit-empty",
    ] {
        let stack = Stack::new("completed").await;
        let accepted = stack.request("child2").await;
        legacy_head_plan(&stack, &accepted.operation_id);
        if variant == "expand" {
            stack.session("new", Some("grandchild1"), "completed");
        }
        {
            let mut store = stack.store.lock().unwrap();
            match variant {
                "non-next" => {
                    store.sessions.remove("child2");
                }
                "foreign" => {
                    store.put(deletion::GUARDS, "grandchild1", json!("foreign"));
                }
                "reparent" => {
                    store.sessions.get_mut("child2").unwrap()["metadata"]["parent_session_id"] =
                        json!("child1");
                }
                "order" => {
                    let mut row = store.state(deletion::OPERATIONS, &accepted.operation_id);
                    row["members"] = json!(["grandchild1", "child2"]);
                    store.put(deletion::OPERATIONS, &accepted.operation_id, row);
                }
                "explicit-null" | "explicit-empty" => {
                    store.sessions.remove("grandchild1");
                    let mut row = store.state(deletion::OPERATIONS, &accepted.operation_id);
                    if variant == "explicit-null" {
                        row["erasing"] = Value::Null;
                    } else {
                        row["links"] = json!([]);
                    }
                    store.put(deletion::OPERATIONS, &accepted.operation_id, row);
                }
                _ => {}
            }
        }
        let failed = stack.run(&accepted.operation_id).await;
        assert_eq!(
            failed.status,
            DeletionStatus::Failed,
            "{variant}: {failed:?}"
        );
        assert!(!failed.force_eligible, "{variant}");
        let store = stack.store.lock().unwrap();
        assert!(
            !store.calls.iter().any(|(f, _)| f == "session::delete"),
            "{variant}"
        );
        assert!(store.sessions.contains_key("parent") && store.sessions.contains_key("child1"));
        if variant == "foreign" {
            assert_eq!(
                store.state(deletion::GUARDS, "grandchild1"),
                json!("foreign")
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review2_returned_unknown_and_restart_witnesses_fail_promptly() {
    for restart in [false, true] {
        let mut stack = Stack::new("completed").await;
        stack
            .store
            .lock()
            .unwrap()
            .codes
            .insert("ext::unknown".into(), "timeout".into());
        let engine = stack.deps.engine().await;
        let reply = harness::functions::subscribe::invoke(
            &stack.deps,
            &engine,
            &harness::policy::CompiledPolicy::from(None),
            "ext::unknown",
            &json!({}),
            "grandchild1",
            false,
            None,
        )
        .await;
        assert!(reply.is_error);
        assert_eq!(dispatch_witnesses(&stack.store.lock().unwrap()).len(), 1);
        if restart {
            stack.reset_runtime();
        }
        let accepted = stack.request("child2").await;
        let started = std::time::Instant::now();
        let failed = stack.run(&accepted.operation_id).await;
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(failed.status, DeletionStatus::Failed);
        assert!(failed
            .blockers
            .iter()
            .any(|b| b.kind == deletion::BlockerKind::UnknownCompletion));
        assert!(!stack
            .store
            .lock()
            .unwrap()
            .calls
            .iter()
            .any(|(f, _)| f == "session::delete"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review2_live_dispatch_deadline_is_active_not_unknown_and_force_keeps_late_cas() {
    let stack = Stack::new("completed").await;
    let gate = Arc::new(Notify::new());
    stack.store.lock().unwrap().hold_target_reply = Some(("ext::live".into(), gate.clone()));
    let deps = stack.deps.clone();
    let dispatch = tokio::spawn(async move {
        let engine = deps.engine().await;
        harness::functions::subscribe::invoke(
            &deps,
            &engine,
            &harness::policy::CompiledPolicy::from(None),
            "ext::live",
            &json!({}),
            "grandchild1",
            false,
            None,
        )
        .await
    });
    stack
        .wait_for(|s| s.calls.iter().any(|(f, _)| f == "ext::live"))
        .await;
    let accepted = stack.request("child2").await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut row = store.state(deletion::OPERATIONS, &accepted.operation_id);
        row["deadline"] = json!(harness::types::message::AgentMessage::now_ms() + 300);
        store.put(deletion::OPERATIONS, &accepted.operation_id, row);
    }
    let failed = stack.run(&accepted.operation_id).await;
    assert_eq!(failed.status, DeletionStatus::Failed);
    assert!(failed.force_eligible);
    assert!(failed
        .blockers
        .iter()
        .all(|b| b.kind == deletion::BlockerKind::ActiveProcessing));
    let forced = force(&stack, &failed).await.unwrap();
    assert_eq!(
        stack.run(&forced.operation_id).await.status,
        DeletionStatus::Completed
    );
    gate.notify_one();
    assert!(dispatch.await.unwrap().is_error);
    assert!(dispatch_witnesses(&stack.store.lock().unwrap()).is_empty());
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .sessions
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["child1", "parent"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review2_legacy_migration_interrupted_reservation_and_intent_save_resume_safely() {
    for lost_intent_ack in [false, true] {
        let stack = Stack::new("completed").await;
        let accepted = stack.request("child2").await;
        legacy_head_plan(&stack, &accepted.operation_id);
        {
            let mut store = stack.store.lock().unwrap();
            if lost_intent_ack {
                store.sessions.remove("grandchild1");
                store.fail_legacy_migration_save_once = true;
            } else {
                store.put(deletion::GUARDS, "grandchild1", Value::Null);
                store.fail_descendant_claim_once = true;
            }
        }
        let failed = stack.run(&accepted.operation_id).await;
        assert_eq!(failed.status, DeletionStatus::Failed);
        assert!(!failed.force_eligible);
        {
            let store = stack.store.lock().unwrap();
            assert!(!store.calls.iter().any(|(f, _)| f == "session::delete"));
            let row = store.state(deletion::OPERATIONS, &accepted.operation_id);
            assert!(row.get("links").is_none());
            if lost_intent_ack {
                assert_eq!(row["erasing"], "grandchild1");
                assert_eq!(
                    row["snapshot"]["unconfirmed_session_ids"],
                    json!(["grandchild1"])
                );
            } else {
                assert!(row.get("erasing").is_none() && row.get("cleanup_started").is_none());
            }
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
                .sessions
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["child1", "parent"]
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review2_runtime_reset_reclassifies_even_a_preexisting_live_ticket_as_unknown() {
    let mut stack = Stack::new("completed").await;
    let gate = Arc::new(Notify::new());
    stack.store.lock().unwrap().hold_target_reply =
        Some(("ext::before-reset".into(), gate.clone()));
    let deps = stack.deps.clone();
    let dispatch = tokio::spawn(async move {
        let engine = deps.engine().await;
        harness::functions::subscribe::invoke(
            &deps,
            &engine,
            &harness::policy::CompiledPolicy::from(None),
            "ext::before-reset",
            &json!({}),
            "grandchild1",
            false,
            None,
        )
        .await
    });
    stack
        .wait_for(|s| s.calls.iter().any(|(f, _)| f == "ext::before-reset"))
        .await;
    stack.reset_runtime();
    let accepted = stack.request("child2").await;
    let started = std::time::Instant::now();
    let failed = stack.run(&accepted.operation_id).await;
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(failed.status, DeletionStatus::Failed);
    assert!(failed
        .blockers
        .iter()
        .any(|b| b.kind == deletion::BlockerKind::UnknownCompletion));
    assert!(!stack
        .store
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|(f, _)| f == "session::delete"));
    gate.notify_one();
    let _ = dispatch.await.unwrap();
    assert!(dispatch_witnesses(&stack.store.lock().unwrap()).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review2_normal_waits_for_witness_admission_before_classifying_an_orphan() {
    let stack = Stack::new("completed").await;
    let gate = Arc::new(Notify::new());
    stack.store.lock().unwrap().hold_dispatch_reply = Some(gate.clone());
    let deps = stack.deps.clone();
    let dispatch = tokio::spawn(async move {
        let engine = deps.engine().await;
        harness::functions::subscribe::invoke(
            &deps,
            &engine,
            &harness::policy::CompiledPolicy::from(None),
            "ext::not-admitted",
            &json!({}),
            "grandchild1",
            false,
            None,
        )
        .await
    });
    stack.wait_for(|s| !dispatch_witnesses(s).is_empty()).await;
    let accepted = stack.request("child2").await;
    let deps = stack.deps.clone();
    let id = accepted.operation_id.clone();
    let runner = tokio::spawn(async move {
        deletion::run(&deps, StatusRequest { operation_id: id })
            .await
            .unwrap()
            .unwrap()
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!runner.is_finished());
    assert!(!stack
        .store
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|(f, _)| f == "session::delete"));
    gate.notify_one();
    assert!(dispatch.await.unwrap().is_error);
    let done = tokio::time::timeout(Duration::from_secs(3), runner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, DeletionStatus::Completed);
    assert!(!stack
        .store
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|(f, _)| f == "ext::not-admitted"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review3_b_normal_waits_for_approved_call_executing_under_session_lock() {
    let stack = Stack::new("completed").await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut turn = store.state("harness_turn", "grandchild1");
        // deferred::resolve(action=execute) marks the released call Triggered
        // and invokes it while holding ONLY the session lock (no turn_activity).
        turn["status"] = json!("awaiting_functions");
        turn["calls"] = json!({"c1": {"state": "triggered", "function_id": "slow::approved"}});
        store.put("harness_turn", "grandchild1", turn);
    }
    let resolve_lock = stack.deps.locks.guard("grandchild1").await;
    let accepted = stack.request("child2").await;
    let deps = stack.deps.clone();
    let id = accepted.operation_id.clone();
    let job = tokio::spawn(async move {
        deletion::run(&deps, StatusRequest { operation_id: id })
            .await
            .unwrap()
            .unwrap()
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    let finished_before_resolve = job.is_finished();
    assert!(
        !finished_before_resolve,
        "normal must wait for the resolve writer"
    );
    // The approved call returns; resolve records Done and the turn ends cancelled.
    {
        let mut store = stack.store.lock().unwrap();
        let mut turn = store.state("harness_turn", "grandchild1");
        turn["status"] = json!("cancelled");
        turn["calls"] = json!({"c1": {"state": "done", "function_id": "slow::approved"}});
        store.put("harness_turn", "grandchild1", turn);
    }
    drop(resolve_lock);
    stack.deps.deletion_changed.notify_waiters();
    let done = tokio::time::timeout(Duration::from_secs(5), job)
        .await
        .unwrap()
        .unwrap();
    println!("R3B approved call under session lock: finished_before_resolve={finished_before_resolve} result={done:?}");
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    assert!(!done.force_eligible && done.blockers.is_empty());
    assert_eq!(done.deleted_session_ids, ["grandchild1", "child2"]);
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .sessions
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["child1", "parent"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review3_b2_approved_call_with_live_dispatch_is_not_unknown() {
    let stack = Stack::new("completed").await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut turn = store.state("harness_turn", "grandchild1");
        turn["status"] = json!("awaiting_functions");
        turn["calls"] = json!({"c1": {"state": "triggered", "function_id": "ext::approved"}});
        store.put("harness_turn", "grandchild1", turn);
    }
    let gate = Arc::new(Notify::new());
    stack.store.lock().unwrap().hold_target_reply = Some(("ext::approved".into(), gate.clone()));
    let resolve_lock = stack.deps.locks.guard("grandchild1").await;
    let deps = stack.deps.clone();
    // Same chokepoint deferred::resolve uses for the released call.
    let dispatch = tokio::spawn(async move {
        let engine = deps.engine().await;
        harness::functions::subscribe::invoke(
            &deps,
            &engine,
            &harness::policy::CompiledPolicy::from(None),
            "ext::approved",
            &json!({}),
            "grandchild1",
            true,
            None,
        )
        .await
    });
    stack
        .wait_for(|s| s.calls.iter().any(|(f, _)| f == "ext::approved"))
        .await;
    let accepted = stack.request("child2").await;
    let started = std::time::Instant::now();
    let deps = stack.deps.clone();
    let id = accepted.operation_id.clone();
    let job = tokio::spawn(async move {
        deletion::run(&deps, StatusRequest { operation_id: id })
            .await
            .unwrap()
            .unwrap()
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    let finished_early = job.is_finished();
    assert!(
        !finished_early,
        "normal must wait for the live approved call"
    );
    gate.notify_one();
    let _ = dispatch.await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut turn = store.state("harness_turn", "grandchild1");
        turn["status"] = json!("cancelled");
        turn["calls"] = json!({"c1": {"state": "done", "function_id": "ext::approved"}});
        store.put("harness_turn", "grandchild1", turn);
    }
    drop(resolve_lock);
    stack.deps.deletion_changed.notify_waiters();
    let done = tokio::time::timeout(Duration::from_secs(5), job)
        .await
        .unwrap()
        .unwrap();
    println!("R3B2 approved call + live dispatch: finished_before_reply={finished_early} elapsed={:?} result={done:?}", started.elapsed());
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    assert!(!done.force_eligible && done.blockers.is_empty());
    assert_eq!(done.deleted_session_ids, ["grandchild1", "child2"]);
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .sessions
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["child1", "parent"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review3_triggered_without_either_writer_or_live_ticket_is_unknown_promptly() {
    let stack = Stack::new("completed").await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut turn = store.state("harness_turn", "grandchild1");
        turn["calls"] = json!({"c1": {"state":"triggered", "function_id":"ext::orphan"}});
        store.put("harness_turn", "grandchild1", turn);
        assert!(dispatch_witnesses(&store).is_empty());
    }
    assert!(stack.deps.locks.try_guard("grandchild1").is_some());
    assert!(stack.deps.turn_activity.try_guard("grandchild1").is_some());
    let accepted = stack.request("child2").await;
    let started = std::time::Instant::now();
    let failed = stack.run(&accepted.operation_id).await;
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(failed.status, DeletionStatus::Failed);
    assert_eq!(failed.blockers.len(), 1);
    let blocker = &failed.blockers[0];
    assert_eq!(blocker.kind, deletion::BlockerKind::UnknownCompletion);
    assert_eq!(blocker.function_id.as_deref(), Some("ext::orphan"));
    assert_eq!(blocker.call_id.as_deref(), Some("c1"));
    assert!(!stack
        .store
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|(f, _)| f == "session::delete"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review3_live_ticket_alone_keeps_triggered_active_and_preserves_callable_diagnostics() {
    let stack = Stack::new("completed").await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut turn = store.state("harness_turn", "grandchild1");
        turn["calls"] =
            json!({"c1": {"state":"triggered", "function_id":"ext::detached-approved"}});
        store.put("harness_turn", "grandchild1", turn);
    }
    let gate = Arc::new(Notify::new());
    stack.store.lock().unwrap().hold_target_reply =
        Some(("ext::detached-approved".into(), gate.clone()));
    let deps = stack.deps.clone();
    let dispatch = tokio::spawn(async move {
        harness::functions::subscribe::invoke(
            &deps,
            &deps.engine().await,
            &harness::policy::CompiledPolicy::from(None),
            "ext::detached-approved",
            &json!({}),
            "grandchild1",
            false,
            None,
        )
        .await
    });
    stack
        .wait_for(|s| s.calls.iter().any(|(f, _)| f == "ext::detached-approved"))
        .await;
    assert!(stack.deps.locks.try_guard("grandchild1").is_some());
    assert!(stack.deps.turn_activity.try_guard("grandchild1").is_some());
    let accepted = stack.request("child2").await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut row = store.state(deletion::OPERATIONS, &accepted.operation_id);
        row["deadline"] = json!(harness::types::message::AgentMessage::now_ms() + 300);
        store.put(deletion::OPERATIONS, &accepted.operation_id, row);
    }
    let failed = stack.run(&accepted.operation_id).await;
    assert_eq!(failed.status, DeletionStatus::Failed);
    assert!(!failed.blockers.is_empty());
    assert!(failed
        .blockers
        .iter()
        .all(|b| b.kind == deletion::BlockerKind::ActiveProcessing));
    let calls: Vec<_> = failed
        .blockers
        .iter()
        .filter(|b| b.call_id.as_deref() == Some("c1"))
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].function_id.as_deref(),
        Some("ext::detached-approved")
    );
    assert!(!stack
        .store
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|(f, _)| f == "session::delete"));
    gate.notify_one();
    assert!(dispatch.await.unwrap().is_error); // confirmed fixture target error, not a timeout
    {
        let mut store = stack.store.lock().unwrap();
        let mut turn = store.state("harness_turn", "grandchild1");
        turn["calls"]["c1"]["state"] = json!("done");
        store.put("harness_turn", "grandchild1", turn);
    }
    let retry = stack.request("child2").await;
    assert_eq!(retry.mode, deletion::DeletionMode::Normal);
    assert_eq!(
        stack.run(&retry.operation_id).await.status,
        DeletionStatus::Completed
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review3_resolve_writer_beyond_full_grace_is_only_active_and_force_never_erases_under_it() {
    let stack = Stack::new("completed").await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut turn = store.state("harness_turn", "grandchild1");
        turn["status"] = json!("awaiting_functions");
        turn["calls"] = json!({"c1": {"state":"triggered", "function_id":"slow::approved"}});
        store.put("harness_turn", "grandchild1", turn);
    }
    let resolve_lock = stack.deps.locks.guard("grandchild1").await;
    assert!(stack.deps.turn_activity.try_guard("grandchild1").is_some());
    let accepted = stack.request("child2").await;
    let started = std::time::Instant::now();
    // Exercise the real 30 s grace, not a shortened deadline or a mocked clock.
    let failed = tokio::time::timeout(
        Duration::from_secs(35),
        deletion::run(
            &stack.deps,
            StatusRequest {
                operation_id: accepted.operation_id,
            },
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert!(started.elapsed() >= Duration::from_secs(30));
    assert_eq!(failed.status, DeletionStatus::Failed);
    assert!(!failed.blockers.is_empty());
    assert!(failed
        .blockers
        .iter()
        .all(|b| b.kind == deletion::BlockerKind::ActiveProcessing));
    let calls: Vec<_> = failed
        .blockers
        .iter()
        .filter(|b| b.call_id.as_deref() == Some("c1"))
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].function_id.as_deref(), Some("slow::approved"));
    let forced = force(&stack, &failed).await.unwrap();
    let force_failed = stack.run(&forced.operation_id).await;
    assert_eq!(force_failed.status, DeletionStatus::Failed);
    assert!(force_failed
        .blockers
        .iter()
        .all(|b| b.kind == deletion::BlockerKind::ActiveProcessing));
    assert!(!stack
        .store
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|(f, _)| f == "session::delete"));
    {
        let mut store = stack.store.lock().unwrap();
        let mut turn = store.state("harness_turn", "grandchild1");
        turn["status"] = json!("cancelled");
        turn["calls"]["c1"]["state"] = json!("done");
        store.put("harness_turn", "grandchild1", turn);
    }
    drop(resolve_lock);
    let retry = force(&stack, &force_failed).await.unwrap();
    let done = stack.run(&retry.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed);
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .sessions
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["child1", "parent"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review3_real_deferred_execute_keeps_normal_waiting_until_release_and_cancelled_step() {
    let stack = Stack::new("completed").await;
    {
        let mut store = stack.store.lock().unwrap();
        let mut turn = store.state("harness_turn", "grandchild1");
        turn["status"] = json!("awaiting_functions");
        turn["options"]["functions"] = json!({"allow":["ext::approved-real"]});
        turn["calls"] = json!({"c1": {"state":"pending", "function_id":"ext::approved-real", "held_by":"approval::gate", "held_arguments":{}}});
        store.put("harness_turn", "grandchild1", turn);
    }
    let gate = Arc::new(Notify::new());
    stack.store.lock().unwrap().hold_target_reply =
        Some(("ext::approved-real".into(), gate.clone()));
    let deps = stack.deps.clone();
    let resolve = tokio::spawn(async move {
        harness::functions::function_resolve::handle(&deps, serde_json::from_value(json!({
            "session_id":"grandchild1", "turn_id":"t_grandchild1", "function_call_id":"c1", "action":"execute",
        })).unwrap()).await.unwrap()
    });
    stack
        .wait_for(|s| s.calls.iter().any(|(f, _)| f == "ext::approved-real"))
        .await;
    assert!(stack.deps.locks.try_guard("grandchild1").is_none());
    assert!(stack.deps.turn_activity.try_guard("grandchild1").is_some());
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .state("harness_turn", "grandchild1")["calls"]["c1"]["state"],
        "triggered"
    );
    let accepted = stack.request("child2").await;
    let deps = stack.deps.clone();
    let job = tokio::spawn(async move {
        deletion::run(
            &deps,
            StatusRequest {
                operation_id: accepted.operation_id,
            },
        )
        .await
        .unwrap()
        .unwrap()
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !job.is_finished(),
        "normal must wait for deferred::resolve, not offer Force"
    );
    gate.notify_one();
    let reply = resolve.await.unwrap();
    assert!(reply.resolved && reply.turn_resumed);
    let row = stack
        .store
        .lock()
        .unwrap()
        .state("harness_turn", "grandchild1");
    assert_eq!(row["calls"]["c1"]["state"], "done");
    assert!(dispatch_witnesses(&stack.store.lock().unwrap()).is_empty());
    // The fixture does not consume enqueues. Deliver the queued production step
    // explicitly; it observes the durable tombstone and finalizes cancellation.
    let mut payload = step("grandchild1");
    payload.step = row["step"].as_u64().unwrap();
    let cancelled = harness::turn_loop::run_step(&stack.deps, payload)
        .await
        .unwrap();
    assert_eq!(
        cancelled.status,
        harness::types::turn::TurnStatus::Cancelled
    );
    let done = tokio::time::timeout(Duration::from_secs(5), job)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    assert!(!done.force_eligible && done.blockers.is_empty());
    assert_eq!(done.deleted_session_ids, ["grandchild1", "child2"]);
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .sessions
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["child1", "parent"]
    );
}

fn seed_paginated_dispatches(stack: &Stack) {
    let mut store = stack.store.lock().unwrap();
    for i in 0..150 {
        store.put(
            deletion::DISPATCHES,
            &format!("a-history-{i:03}"),
            Value::Null,
        );
    }
    // Later-page tests must span non-null rows, not filtered retired history.
    for i in 0..150 {
        store.put(
            deletion::DISPATCHES,
            &format!("b-foreign-{i:03}"),
            json!({"session_id":"parent","function_id":"foreign::active"}),
        );
    }
    for (key, sid) in [
        ("z-target-child", "child2"),
        ("z-target-grandchild", "grandchild1"),
        ("z-foreign", "parent"),
    ] {
        store.put(
            deletion::DISPATCHES,
            key,
            json!({"session_id":sid,"function_id":"slow::paged"}),
        );
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pagination_force_cleans_later_target_witnesses_and_preserves_foreign_without_gets() {
    let stack = Stack::new("completed").await;
    seed_paginated_dispatches(&stack);
    let accepted = stack.request("child2").await;
    let normal = stack.run(&accepted.operation_id).await;
    assert!(normal.force_eligible);
    let escalated = force(&stack, &normal).await.unwrap();
    let before = stack.store.lock().unwrap().calls.len();
    let done = stack.run(&escalated.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    let store = stack.store.lock().unwrap();
    assert_eq!(
        store.sessions.keys().cloned().collect::<Vec<_>>(),
        ["child1", "parent"]
    );
    for key in ["z-target-child", "z-target-grandchild"] {
        assert!(store.state(deletion::DISPATCHES, key).is_null());
    }
    assert_eq!(
        store.state(deletion::DISPATCHES, "z-foreign")["session_id"],
        "parent"
    );
    let calls = &store.calls[before..];
    assert_eq!(
        calls
            .iter()
            .filter(|(f, _)| f == "harness::state::list_entries")
            .count(),
        2
    );
    assert!(calls
        .iter()
        .any(|(f, d)| f == "harness::state::list_entries" && d["cursor"] == "page:100"));
    assert!(!calls.iter().any(|(f, d)| (f == "harness::state::get"
        && d["scope"] == deletion::DISPATCHES)
        || f == "state::list_entries"));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pagination_corrupt_later_pages_fail_before_any_destructive_effects() {
    for variant in [
        "duplicate",
        "loop",
        "nonprogress",
        "missing-terminal",
        "missing-cursor",
        "false-terminal",
        "bad-total",
        "bad-offset",
        "bad-row",
        "bad-membership",
        "malformed-target",
        "oversized-row",
        "remote-expiry",
        "remote-huge",
    ] {
        let stack = Stack::new("completed").await;
        seed_paginated_dispatches(&stack);
        let accepted = stack.request("child2").await;
        let normal = stack.run(&accepted.operation_id).await;
        let escalated = force(&stack, &normal).await.unwrap();
        let before;
        {
            let mut store = stack.store.lock().unwrap();
            before = store.calls.len();
            let all = store
                .state
                .iter()
                .filter(|((s, _), v)| s == deletion::DISPATCHES && !v.is_null())
                .map(|((_, k), v)| json!([k, v]))
                .collect::<Vec<_>>();
            let mut page = json!({"entries":&all[100..],"offset":100,"total":all.len(),"done":true,"next_cursor":null});
            match variant {
                "duplicate" => page["entries"][0] = all[0].clone(),
                "loop" => {
                    page["entries"] = json!([all[100]]);
                    page["done"] = json!(false);
                    page["next_cursor"] = json!("page:100");
                }
                "nonprogress" => {
                    page["entries"] = json!([]);
                    page["done"] = json!(false);
                    page["next_cursor"] = json!("page:101");
                }
                "missing-terminal" => {
                    page.as_object_mut().unwrap().remove("done");
                }
                "missing-cursor" => {
                    page.as_object_mut().unwrap().remove("next_cursor");
                }
                "false-terminal" => {
                    page["done"] = json!(false);
                }
                "bad-total" => page["total"] = json!(all.len() + 1),
                "bad-offset" => page["offset"] = json!(99),
                "bad-row" => page["entries"][0] = json!([42, {}]),
                "bad-membership" => page["entries"][0] = json!(["bad",{"function_id":"no-owner"}]),
                "malformed-target" => {
                    page["entries"][0] =
                        json!(["bad-target",{"session_id":"grandchild1","function_id":42}])
                }
                "oversized-row" => {
                    page["entries"][0] = json!(["big",{"session_id":"grandchild1","function_id":"x".repeat(1_000_000)}])
                }
                "remote-expiry" => {
                    store.fail_entry_page = Some(100);
                    store.codes.insert(
                        "harness::state::list_entries".into(),
                        "INVALID_CURSOR".into(),
                    );
                }
                "remote-huge" => store.fail_entry_page = Some(100),
                _ => unreachable!(),
            }
            if !variant.starts_with("remote") {
                store.entry_page_overrides.insert(100, page);
            }
        }
        let failed = stack.run(&escalated.operation_id).await;
        assert_eq!(
            failed.status,
            DeletionStatus::Failed,
            "{variant}: {failed:?}"
        );
        assert!(!failed.force_eligible, "{variant}");
        assert!(failed.deleted_session_ids.is_empty(), "{variant}");
        let store = stack.store.lock().unwrap();
        assert_eq!(store.sessions.len(), 4, "{variant}");
        assert!(
            !store.calls[before..]
                .iter()
                .any(|(f, d)| f == "session::delete"
                    || f == "session::append"
                    || (f == "harness::state::compare-and-set"
                        && d["scope"] == deletion::DISPATCHES
                        && d["value"].is_null())),
            "{variant}"
        );
        for key in ["z-target-child", "z-target-grandchild", "z-foreign"] {
            assert!(
                !store.state(deletion::DISPATCHES, key).is_null(),
                "{variant}"
            );
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pagination_normal_transport_failure_does_not_offer_force() {
    let stack = Stack::new("completed").await;
    seed_paginated_dispatches(&stack);
    stack.store.lock().unwrap().fail_entry_page = Some(100);
    let accepted = stack.request("child2").await;
    let failed = stack.run(&accepted.operation_id).await;
    assert_eq!(failed.status, DeletionStatus::Failed);
    assert!(!failed.force_eligible);
    assert!(failed.deleted_session_ids.is_empty());
    assert_eq!(stack.store.lock().unwrap().sessions.len(), 4);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pagination_later_page_respects_original_durable_deadline() {
    let stack = Stack::new("completed").await;
    seed_paginated_dispatches(&stack);
    let accepted = stack.request("child2").await;
    let normal = stack.run(&accepted.operation_id).await;
    let escalated = force(&stack, &normal).await.unwrap();
    let gate = Arc::new(Notify::new());
    let before;
    {
        let mut store = stack.store.lock().unwrap();
        before = store.calls.len();
        let mut row = store.state(deletion::OPERATIONS, &normal.operation_id);
        row["deadline"] = json!(harness::types::message::AgentMessage::now_ms() + 350);
        store.put(deletion::OPERATIONS, &normal.operation_id, row);
        store.hold_later_entry_reply = Some(gate.clone());
    }
    let started = std::time::Instant::now();
    let failed = stack.run(&escalated.operation_id).await;
    gate.notify_one();
    assert_eq!(failed.status, DeletionStatus::Failed);
    assert!(!failed.force_eligible);
    assert!(failed.deleted_session_ids.is_empty());
    assert!(started.elapsed() < Duration::from_secs(2));
    let store = stack.store.lock().unwrap();
    assert_eq!(store.sessions.len(), 4);
    assert!(store.calls[before..]
        .iter()
        .any(|(f, d)| f == "harness::state::list_entries" && d["cursor"] == "page:100"));
    assert!(!store.calls[before..]
        .iter()
        .any(|(f, _)| f == "session::delete"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pagination_expired_attempt_can_be_diagnosed_without_rewriting_its_durable_deadline() {
    let stack = Stack::new("completed").await;
    seed_paginated_dispatches(&stack);
    let accepted = stack.request("child2").await;
    let failed = stack.run(&accepted.operation_id).await;
    assert!(failed.force_eligible);
    let expired = harness::types::message::AgentMessage::now_ms() - 1;
    {
        let mut store = stack.store.lock().unwrap();
        let mut row = store.state(deletion::OPERATIONS, &accepted.operation_id);
        row["deadline"] = json!(expired);
        store.put(deletion::OPERATIONS, &accepted.operation_id, row);
    }
    let diagnosed = deletion::status(
        &stack.deps,
        deletion::StatusRequest {
            operation_id: accepted.operation_id.clone(),
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert!(diagnosed.force_eligible);
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .state(deletion::OPERATIONS, &accepted.operation_id)["deadline"],
        json!(expired)
    );
    let forced = force(&stack, &diagnosed).await.unwrap();
    assert_eq!(
        stack.run(&forced.operation_id).await.status,
        DeletionStatus::Completed
    );
    assert_eq!(
        stack
            .store
            .lock()
            .unwrap()
            .sessions
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["child1", "parent"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pagination_failed_diagnosis_cannot_confirm_force_for_an_expired_attempt() {
    let stack = Stack::new("completed").await;
    seed_paginated_dispatches(&stack);
    let accepted = stack.request("child2").await;
    let failed = stack.run(&accepted.operation_id).await;
    assert!(failed.force_eligible);
    let expired = harness::types::message::AgentMessage::now_ms() - 1;
    let before;
    {
        let mut store = stack.store.lock().unwrap();
        let mut row = store.state(deletion::OPERATIONS, &accepted.operation_id);
        row["deadline"] = json!(expired);
        store.put(deletion::OPERATIONS, &accepted.operation_id, row);
        store.fail_entry_page = Some(100);
        before = store.calls.len();
    }
    let diagnosed = deletion::status(
        &stack.deps,
        deletion::StatusRequest {
            operation_id: accepted.operation_id.clone(),
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!diagnosed.force_eligible);
    assert!(force(&stack, &failed).await.is_err());
    let store = stack.store.lock().unwrap();
    assert_eq!(
        store.state(deletion::OPERATIONS, &accepted.operation_id)["deadline"],
        json!(expired)
    );
    assert_eq!(
        store.state(deletion::OPERATIONS, &accepted.operation_id)["snapshot"]["mode"],
        json!("normal")
    );
    assert_eq!(store.sessions.len(), 4);
    assert!(!store.calls[before..]
        .iter()
        .any(|(f, d)| f == "session::delete"
            || (f == "harness::state::compare-and-set"
                && d["scope"] == deletion::DISPATCHES
                && d["value"].is_null())));
}

fn seed_b1_retired_history(stack: &Stack) {
    let mut store = stack.store.lock().unwrap();
    for i in 0..100_001 {
        store.put(
            deletion::DISPATCHES,
            &format!("retired-b1-{i:06}"),
            Value::Null,
        );
    }
}

fn assert_b1_history_and_scan_contract(store: &Store, calls: &[(String, Value)]) {
    assert_eq!(
        store
            .state
            .iter()
            .filter(|((s, k), v)| s == deletion::DISPATCHES
                && k.starts_with("retired-b1-")
                && v.is_null())
            .count(),
        100_001
    );
    let scans = calls
        .iter()
        .filter(|(f, _)| f == "harness::state::list_entries")
        .collect::<Vec<_>>();
    assert!(!scans.is_empty());
    assert!(scans.iter().all(|(_, d)| d["scope"] == deletion::DISPATCHES
        && d["non_null_only"] == true
        && d["limit"] == 100
        && d["max_bytes"] == 1_000_000));
    assert!(!calls.iter().any(|(f, d)| f == "state::list_entries"
        || (f == "harness::state::get" && d["scope"] == deletion::DISPATCHES)
        || (f.ends_with("::delete") && d["scope"] == deletion::DISPATCHES)));
    assert!(calls
        .iter()
        .all(|(_, d)| d.get("non_null_only").is_none() || d["scope"] == deletion::DISPATCHES));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pagination_b1_retired_history_does_not_block_normal_deletion_or_change_tombstones() {
    let stack = Stack::new("completed").await;
    seed_b1_retired_history(&stack);
    {
        let mut store = stack.store.lock().unwrap();
        for i in 0..150 {
            store.put(
                deletion::DISPATCHES,
                &format!("foreign-b1-{i:03}"),
                json!({"session_id":"parent","function_id":"foreign::live"}),
            );
        }
    }
    let accepted = stack.request("child2").await;
    let done = stack.run(&accepted.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    assert_eq!(
        deletion::status(
            &stack.deps,
            StatusRequest {
                operation_id: accepted.operation_id.clone()
            }
        )
        .await
        .unwrap(),
        Some(done)
    );
    let store = stack.store.lock().unwrap();
    assert_b1_history_and_scan_contract(&store, &store.calls);
    assert_eq!(
        store.sessions.keys().cloned().collect::<Vec<_>>(),
        ["child1", "parent"]
    );
    for id in ["child2", "grandchild1"] {
        assert_eq!(
            store.state(deletion::GUARDS, id),
            json!(accepted.operation_id)
        );
    }
    for i in 0..150 {
        assert_eq!(
            store.state(deletion::DISPATCHES, &format!("foreign-b1-{i:03}"))["session_id"],
            "parent"
        );
    }
    println!("B1 Harness normal: 100001 retired keys retained; foreign rows, parent/sibling and deletion tombstones preserved");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pagination_b1_history_status_and_force_find_later_targets_without_mutating_retired_keys() {
    let stack = Stack::new("completed").await;
    seed_paginated_dispatches(&stack);
    seed_b1_retired_history(&stack);
    let accepted = stack.request("child2").await;
    let failed = stack.run(&accepted.operation_id).await;
    assert_eq!(failed.status, DeletionStatus::Failed);
    assert!(failed.force_eligible);
    assert!(failed.blockers.iter().any(
        |b| b.session_id == "grandchild1" && b.kind == deletion::BlockerKind::UnknownCompletion
    ));
    let status = deletion::status(
        &stack.deps,
        StatusRequest {
            operation_id: accepted.operation_id.clone(),
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert!(status.force_eligible);
    let forced = force(&stack, &status).await.unwrap();
    let before = stack.store.lock().unwrap().calls.len();
    let done = stack.run(&forced.operation_id).await;
    assert_eq!(done.status, DeletionStatus::Completed, "{done:?}");
    let store = stack.store.lock().unwrap();
    assert_b1_history_and_scan_contract(&store, &store.calls);
    assert_eq!(
        store.sessions.keys().cloned().collect::<Vec<_>>(),
        ["child1", "parent"]
    );
    assert!(store
        .state
        .contains_key(&(deletion::DISPATCHES.into(), "z-target-child".into())));
    assert!(store
        .state
        .contains_key(&(deletion::DISPATCHES.into(), "z-target-grandchild".into())));
    for key in ["z-target-child", "z-target-grandchild"] {
        assert!(store.state(deletion::DISPATCHES, key).is_null());
    }
    assert_eq!(
        store.state(deletion::DISPATCHES, "z-foreign")["session_id"],
        "parent"
    );
    let calls = &store.calls[before..];
    assert_eq!(
        calls
            .iter()
            .filter(|(f, _)| f == "harness::state::list_entries")
            .count(),
        2
    );
    let terminal = calls
        .iter()
        .position(|(f, d)| f == "harness::state::list_entries" && d["cursor"] == "page:100")
        .unwrap();
    assert!(calls
        .iter()
        .enumerate()
        .filter(|(_, (f, d))| f == "session::delete"
            || (f == "harness::state::compare-and-set"
                && d["scope"] == deletion::DISPATCHES
                && d["value"].is_null()))
        .all(|(index, _)| index > terminal));
    for id in ["child2", "grandchild1"] {
        assert_eq!(
            store.state(deletion::GUARDS, id),
            json!(accepted.operation_id)
        );
    }
    println!("B1 Harness status/Force: >100k retired keys, 153 non-null witnesses over two pages, later targets CAS-purged after complete scan, foreign preserved");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pagination_b1_history_never_filters_ambiguous_non_null_or_allows_failed_confirmation() {
    for ambiguous in [
        json!({"function_id":"no-owner"}),
        json!({"session_id":"grandchild1","function_id":42}),
    ] {
        let stack = Stack::new("completed").await;
        seed_paginated_dispatches(&stack);
        seed_b1_retired_history(&stack);
        let accepted = stack.request("child2").await;
        let failed = stack.run(&accepted.operation_id).await;
        assert!(failed.force_eligible);
        {
            let mut store = stack.store.lock().unwrap();
            store.put(deletion::DISPATCHES, "z-malformed-b1", ambiguous.clone());
        }
        let before = stack.store.lock().unwrap().calls.len();
        let status = deletion::status(
            &stack.deps,
            StatusRequest {
                operation_id: accepted.operation_id.clone(),
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert!(!status.force_eligible);
        assert!(force(&stack, &failed).await.is_err());
        let normal = stack.request("child2").await;
        let rejected = stack.run(&normal.operation_id).await;
        assert_eq!(rejected.status, DeletionStatus::Failed);
        assert!(!rejected.force_eligible);
        let store = stack.store.lock().unwrap();
        assert_b1_history_and_scan_contract(&store, &store.calls);
        assert_eq!(store.sessions.len(), 4);
        assert_eq!(
            store.state(deletion::DISPATCHES, "z-malformed-b1"),
            ambiguous
        );
        assert!(!store.calls[before..]
            .iter()
            .any(|(f, d)| f == "session::delete"
                || f == "session::append"
                || (f == "harness::state::compare-and-set"
                    && d["scope"] == deletion::DISPATCHES
                    && d["value"].is_null())));
    }
}
