//! The harness repairs the coarse status of sessions IT runs: one the store
//! still reports `working` with no turn record behind it is set back to
//! `idle`. A session another agent worker runs (`metadata.agent`, e.g. a
//! claude-code sub-agent streaming its own transcript) has no harness turn
//! record by design, so that repair would flip a live `working` to `idle` —
//! and the console would bounce the pane's live state while the agent kept
//! writing. These tests pin that the harness writes NOTHING to such a session:
//! no `idle`, and no corrective write either.
//!
//! Executes the production handlers through an isolated mock SDK transport.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use harness::config::WorkerConfig;
use harness::deps::Deps;
use harness::functions::status::{self, StatusRequest};
use iii_sdk::{InitOptions, register_worker};
use serde_json::{Value, json};
use tokio::sync::{RwLock, oneshot};

mod support;
use support::host;

#[derive(Default)]
struct Store {
    /// session id -> session meta (`status`, `metadata`).
    sessions: BTreeMap<String, Value>,
    calls: Vec<(String, Value)>,
}

impl Store {
    fn respond(&mut self, function: &str, data: &Value) -> Result<Value, String> {
        self.calls.push((function.into(), data.clone()));
        let sid = data["session_id"].as_str().unwrap_or_default();
        match function {
            // No harness turn record for any session here.
            "state::get" | "harness::state::get" => Ok(Value::Null),
            "session::get" => Ok(self
                .sessions
                .get(sid)
                .map(|meta| json!({ "meta": meta }))
                .unwrap_or(Value::Null)),
            "session::list" => {
                let wanted = data["status"].clone();
                let sessions: Vec<Value> = self
                    .sessions
                    .values()
                    .filter(|meta| wanted.is_null() || meta["status"] == wanted)
                    .cloned()
                    .collect();
                Ok(json!({ "sessions": sessions, "next_cursor": null }))
            }
            "session::set-status" => {
                if let Some(meta) = self.sessions.get_mut(sid) {
                    meta["status"] = data["status"].clone();
                }
                Ok(json!({ "ok": true }))
            }
            _ => Err(format!("unexpected mock RPC {function}: {data}")),
        }
    }

    fn status_writes(&self, sid: &str) -> Vec<Value> {
        self.calls
            .iter()
            .filter(|(f, d)| f == "session::set-status" && d["session_id"] == sid)
            .map(|(_, d)| d["status"].clone())
            .collect()
    }
}

struct Stack {
    deps: Deps,
    store: Arc<Mutex<Store>>,
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
        let store = Arc::new(Mutex::new(Store::default()));
        let (ready_tx, ready_rx) = oneshot::channel();
        let state = store.clone();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let text = |value: Value| {
                tokio_tungstenite::tungstenite::Message::Text(value.to_string().into())
            };
            let mut engine = host::Host::default();
            socket.send(text(host::hello())).await.unwrap();
            let _ = ready_tx.send(());
            while let Some(Ok(frame)) = socket.next().await {
                let Ok(raw) = frame.to_text() else { continue };
                let Ok(message) = serde_json::from_str::<Value>(raw) else {
                    continue;
                };
                if let Some(answer) = engine.answer(&message) {
                    socket.send(text(answer)).await.unwrap();
                    continue;
                }
                if message["type"] != "invokefunction" {
                    continue;
                }
                let function = message["function_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let result = state.lock().unwrap().respond(&function, &message["data"]);
                if message["invocation_id"].is_null() {
                    continue;
                }
                let mut reply = json!({
                    "type": "invocationresult",
                    "invocation_id": message["invocation_id"],
                    "function_id": function,
                });
                match result {
                    Ok(value) => reply["result"] = value,
                    Err(error) => {
                        reply["error"] = json!({ "code": "test_error", "message": error })
                    }
                }
                if socket.send(text(reply)).await.is_err() {
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
        Self {
            deps,
            store,
            server,
        }
    }

    fn session(&self, id: &str, status: &str, metadata: Value) {
        self.store.lock().unwrap().sessions.insert(
            id.into(),
            json!({ "session_id": id, "status": status, "metadata": metadata }),
        );
    }

    fn status_writes(&self, id: &str) -> Vec<Value> {
        self.store.lock().unwrap().status_writes(id)
    }

    /// Wait for background work spawned by a handler to settle.
    async fn settle(&self) {
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

const CLAUDE_CHILD: &str = "claude-child";
const HARNESS_CHAT: &str = "harness-chat";

fn claude_child_metadata() -> Value {
    json!({ "agent": "claude-code", "parent_session_id": HARNESS_CHAT })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconcile_leaves_a_working_session_another_agent_runs_untouched() {
    let stack = Stack::new().await;
    stack.session(CLAUDE_CHILD, "working", claude_child_metadata());

    let wrote = harness::session_status::reconcile(&stack.deps, CLAUDE_CHILD)
        .await
        .expect("reconcile succeeds");

    assert!(!wrote, "no status write for a session another agent runs");
    assert_eq!(stack.status_writes(CLAUDE_CHILD), Vec::<Value>::new());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn harness_status_on_a_running_agent_child_never_flips_it() {
    // The parent chat's sub-agent chip asks `harness::status` about the child
    // while it streams: that read must not touch the child's status at all.
    let stack = Stack::new().await;
    stack.session(CLAUDE_CHILD, "working", claude_child_metadata());

    for _ in 0..3 {
        let report = status::handle(
            &stack.deps,
            StatusRequest {
                session_id: CLAUDE_CHILD.into(),
                verbose: false,
            },
        )
        .await
        .expect("status succeeds");
        assert!(report.is_none(), "the harness has no turn for it");
    }
    stack.settle().await;

    assert_eq!(stack.status_writes(CLAUDE_CHILD), Vec::<Value>::new());
    assert_eq!(
        stack.store.lock().unwrap().sessions[CLAUDE_CHILD]["status"],
        "working"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn boot_sweep_skips_sessions_another_agent_runs() {
    let stack = Stack::new().await;
    stack.session(CLAUDE_CHILD, "working", claude_child_metadata());
    stack.session(HARNESS_CHAT, "working", json!({ "surface": "console" }));

    harness::session_status::sweep(&stack.deps).await;

    assert_eq!(stack.status_writes(CLAUDE_CHILD), Vec::<Value>::new());
    assert_eq!(stack.status_writes(HARNESS_CHAT), vec![json!("idle")]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_harness_session_with_no_record_is_still_repaired_to_idle() {
    // The repair this exists for is unchanged for the harness's own sessions.
    let stack = Stack::new().await;
    stack.session(HARNESS_CHAT, "working", json!({ "surface": "console" }));

    let wrote = harness::session_status::reconcile(&stack.deps, HARNESS_CHAT)
        .await
        .expect("reconcile succeeds");

    assert!(wrote);
    assert_eq!(stack.status_writes(HARNESS_CHAT), vec![json!("idle")]);
}
