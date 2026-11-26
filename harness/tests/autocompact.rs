//! Autocompact (chat-projects phase 3) through the production step handler,
//! over an isolated mock engine (the `support::host` transport, as in
//! `delete_session_tree.rs`): stub session-manager, state, context-manager,
//! router and tmux answers stand in for the workers, and observers of
//! `harness::on-compaction` and `harness::on-turn-completed` for the clients.
//! A held reply stands for a summariser still working. No engine, model or
//! provider runs.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use harness::autocompact::{self, LogRequest, ModelsRequest};
use harness::config::{AutocompactConfig, AutocompactMode, WorkerConfig};
use harness::deps::Deps;
use harness::events::{ON_COMPACTION_FN, ON_TURN_COMPLETED_FN};
use iii_sdk::{InitOptions, register_worker};
use serde_json::{Value, json};
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

mod support;
use support::host;

const S: &str = "s-main";
const SIDE: &str = "s-side";
const CHILD: &str = "s-child";
const TURN: &str = "t-auto";
const MODEL: &str = "claude-code/claude-opus-5-5";
const ACME: &str = "prj_acme";
const INSTRUCTIONS: &str = "keep the ticket ids";
/// The refreshed window after a summary, with [`snapshot`]'s prompt and
/// tools (16k) and `context::count-tokens` answering [`COUNTED`].
const COUNTED: u64 = 18_000;
const AFTER: u64 = 16_000 + COUNTED;

#[derive(Clone, Default)]
struct Store {
    state: BTreeMap<(String, String), Value>,
    /// `session::get`'s `meta` by session id.
    sessions: BTreeMap<String, Value>,
    messages: BTreeMap<String, Vec<Value>>,
    /// Every worker -> engine invocation: `(function_id, data, action)`.
    calls: Vec<(String, Value, Value)>,
    /// What the observers heard: `(observed function, its event)`.
    events: Vec<(String, Value)>,
    observers: BTreeMap<String, String>,
    compact_reply: Value,
    assemble_reply: Value,
    count_tokens: u64,
    tmux: Option<Value>,
    catalog: BTreeMap<String, Value>,
    /// A function whose replies are held until [`Stack::release`].
    hold: Option<&'static str>,
    held: Vec<(Value, String, Value)>,
    /// An entry another harness adds to the log between this one's read
    /// and its compare-and-swap.
    log_interference: Option<Value>,
    unexpected: Vec<String>,
}

impl Store {
    fn get(&self, scope: &str, key: &str) -> Value {
        self.state
            .get(&(scope.into(), key.into()))
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn put(&mut self, scope: &str, key: &str, value: Value) {
        if value.is_null() {
            self.state.remove(&(scope.into(), key.into()));
        } else {
            self.state.insert((scope.into(), key.into()), value);
        }
    }

    fn respond(&mut self, function: &str, data: &Value, action: &Value) -> Result<Value, String> {
        self.calls
            .push((function.into(), data.clone(), action.clone()));
        if action["type"] == "enqueue" {
            return Ok(json!({ "message_receipt_id": "receipt" }));
        }
        self.answer(function, data)
    }

    fn answer(&mut self, function: &str, data: &Value) -> Result<Value, String> {
        let scope = data["scope"].as_str().unwrap_or_default().to_string();
        let key = data["key"].as_str().unwrap_or_default().to_string();
        let sid = data["session_id"].as_str().unwrap_or_default().to_string();
        match function {
            "state::get" | "harness::state::get" => Ok(self.get(&scope, &key)),
            "state::set" | "harness::state::set" => {
                self.put(&scope, &key, data["value"].clone());
                Ok(json!({}))
            }
            "state::delete" | "harness::state::delete" => {
                self.put(&scope, &key, Value::Null);
                Ok(json!({}))
            }
            "state::list" | "harness::state::list" => Ok(Value::Array(
                self.state
                    .iter()
                    .filter(|((s, _), _)| *s == scope)
                    .map(|(_, v)| v.clone())
                    .collect(),
            )),
            "state::list_keys" => Ok(json!({ "keys": self
                .state
                .keys()
                .filter(|(s, _)| *s == scope)
                .map(|(_, k)| k.clone())
                .collect::<Vec<_>>() })),
            // The usage counters (harness_usage) go through the public one.
            "harness::state::compare-and-set" | "state::compare-and-set" => {
                if scope == autocompact::LOG_SCOPE
                    && let Some(entry) = self.log_interference.take()
                {
                    let mut ring = self.get(&scope, &key);
                    if ring.is_null() {
                        ring = json!({ "entries": [] });
                    }
                    ring["entries"].as_array_mut().unwrap().push(entry);
                    self.put(&scope, &key, ring);
                }
                let current = self.get(&scope, &key);
                let swapped = current == data["expected"];
                if swapped {
                    self.put(&scope, &key, data["value"].clone());
                }
                Ok(json!({ "swapped": swapped, "current": current }))
            }
            "state::claim-namespace" => Ok(json!({ "claimed": true })),
            // Registration and the durable topic publishes: nothing to answer.
            "engine::workers::register" | "iii::durable::publish" => Ok(json!({})),
            "session::get" => Ok(self
                .sessions
                .get(&sid)
                .map(|meta| json!({ "meta": meta }))
                .unwrap_or(Value::Null)),
            "session::list" => {
                // session-manager's metadata filter: every key named, equal.
                let filter = data["metadata"].as_object().cloned().unwrap_or_default();
                Ok(json!({ "sessions": self
                    .sessions
                    .values()
                    .filter(|meta| filter.iter().all(|(k, v)| meta["metadata"].get(k) == Some(v)))
                    .cloned()
                    .collect::<Vec<_>>() }))
            }
            "session::ensure" => Ok(json!({
                "meta": self.sessions.get(&sid).cloned().unwrap_or(Value::Null),
                "created": false,
            })),
            "session::messages" => Ok(json!({
                "messages": self.messages.get(&sid).cloned().unwrap_or_default(),
                "next_cursor": null,
            })),
            "session::messages-tail" => Ok(json!({
                "messages": self
                    .messages
                    .get(&sid)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|entry| !entry["message"].is_null())
                    .collect::<Vec<_>>(),
            })),
            "session::append" => {
                let entry_id = data["entry_id"].clone();
                let entries = self.messages.entry(sid).or_default();
                if !entries.iter().any(|entry| entry["entry_id"] == entry_id) {
                    entries.push(json!({
                        "entry_id": entry_id, "message": data["message"], "custom": data["custom"],
                    }));
                }
                Ok(json!({ "entry_id": entry_id }))
            }
            "session::set-status" => Ok(json!({ "ok": true })),
            "session::patch-meta" => Ok(json!({
                "meta": self.sessions.get(&sid).cloned().unwrap_or(Value::Null),
            })),
            "session::fork" => {
                let id = "s-fork";
                self.sessions
                    .insert(id.into(), json!({ "session_id": id, "metadata": {} }));
                Ok(json!({ "session_id": id, "meta": { "message_count": 2 } }))
            }
            "context::compact" => Ok(self.compact_reply.clone()),
            "context::count-tokens" => {
                let tokens = self.count_tokens;
                Ok(json!({
                    "tokens": tokens,
                    "by_role": { "user": tokens / 4, "assistant": tokens / 4,
                                 "function_result": 0, "custom": 0 },
                }))
            }
            "context::assemble" => {
                let mut reply = self.assemble_reply.clone();
                reply["system_prompt"] = json!(data["system_prompt"].as_str().unwrap_or_default());
                reply["messages"] = data["messages"].clone();
                Ok(reply)
            }
            "router::models::get" => Ok(self
                .catalog
                .get(data["id"].as_str().unwrap_or_default())
                .map(|model| json!({ "model": model }))
                .unwrap_or(Value::Null)),
            "tmux::accounts::list" => self
                .tmux
                .clone()
                .ok_or_else(|| "function_not_found: tmux::accounts::list".to_string()),
            // The step's boundary, the stream the provider call opens: what
            // these tests check is decided and stored before it.
            "engine::channels::create" => {
                Err("intentional boundary stop before the provider".into())
            }
            "shell::kill-session-jobs" => Ok(json!({ "killed": [] })),
            "router::abort" => Ok(json!({ "aborted": true })),
            "engine::observers::dispose" => Ok(json!({ "disposed": true })),
            "approval::get-settings" => Ok(json!({ "source": "defaults" })),
            _ => {
                self.unexpected.push(function.to_string());
                Err(format!("unexpected mock RPC {function}"))
            }
        }
    }

    /// What `function` was sent, in order.
    fn sent(&self, function: &str) -> Vec<Value> {
        self.calls
            .iter()
            .filter(|(f, _, _)| f == function)
            .map(|(_, data, _)| data.clone())
            .collect()
    }

    /// The custom rows of `session_id`'s transcript, with their entry ids.
    fn customs(&self, session_id: &str) -> Vec<(String, Value)> {
        self.messages
            .get(session_id)
            .into_iter()
            .flatten()
            .filter(|entry| !entry["custom"].is_null())
            .map(|entry| {
                (
                    entry["entry_id"].as_str().unwrap_or_default().to_string(),
                    entry["custom"].clone(),
                )
            })
            .collect()
    }

    fn custom(&self, session_id: &str, custom_type: &str, reason: Option<&str>) -> Option<Value> {
        self.customs(session_id)
            .into_iter()
            .map(|(_, custom)| custom)
            .find(|custom| {
                custom["custom_type"] == custom_type
                    && reason.is_none_or(|reason| custom["data"]["reason"] == reason)
            })
    }

    fn events_of(&self, function: &str) -> Vec<Value> {
        self.events
            .iter()
            .filter(|(f, _)| f == function)
            .map(|(_, event)| event.clone())
            .collect()
    }
}

struct Stack {
    deps: Deps,
    store: Arc<Mutex<Store>>,
    push: mpsc::UnboundedSender<Value>,
    server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

impl Drop for Stack {
    fn drop(&mut self) {
        self.deps.iii.shutdown();
        self.server.abort();
    }
}

impl Stack {
    async fn new(store: Store, projects: Value, global: AutocompactConfig) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let projects_file = dir.path().join("harness-projects.json");
        std::fs::write(
            &projects_file,
            serde_json::to_vec(&json!({ "version": 2, "revision": 1, "projects": projects }))
                .unwrap(),
        )
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let store = Arc::new(Mutex::new(store));
        let (ready_tx, ready_rx) = oneshot::channel();
        let (push, mut pushed) = mpsc::unbounded_channel::<Value>();
        let state = store.clone();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let text = |value: Value| Message::Text(value.to_string().into());
            let mut engine = host::Host::default();
            socket.send(text(host::hello())).await.unwrap();
            let _ = ready_tx.send(());
            loop {
                let frame = tokio::select! {
                    Some(message) = pushed.recv() => {
                        // A held reply, released.
                        let outgoing = if message["type"] == "invocationresult" {
                            Some(message)
                        } else {
                            engine.announce(message)
                        };
                        if let Some(message) = outgoing {
                            socket.send(text(message)).await.unwrap();
                        }
                        continue;
                    }
                    frame = socket.next() => match frame {
                        Some(Ok(frame)) => frame,
                        _ => break,
                    },
                };
                let Ok(raw) = frame.to_text() else {
                    continue;
                };
                let Ok(message) = serde_json::from_str::<Value>(raw) else {
                    continue;
                };
                if let Some(answer) = engine.answer(&message) {
                    socket.send(text(answer)).await.unwrap();
                    continue;
                }
                let activations = host::activations(&message);
                if !activations.is_empty() {
                    let mut store = state.lock().unwrap();
                    for (id, observed) in activations {
                        if let Some(function) = store.observers.get(&id).cloned() {
                            store.events.push((function, observed));
                        }
                    }
                    continue;
                }
                if message["type"] != "invokefunction" {
                    continue;
                }
                let function = message["function_id"].as_str().unwrap().to_string();
                let result = {
                    let mut store = state.lock().unwrap();
                    if store.hold == Some(function.as_str()) {
                        store.calls.push((
                            function.clone(),
                            message["data"].clone(),
                            message["action"].clone(),
                        ));
                        store.held.push((
                            message["invocation_id"].clone(),
                            function,
                            message["data"].clone(),
                        ));
                        continue;
                    }
                    store.respond(&function, &message["data"], &message["action"])
                };
                if message["invocation_id"].is_null() {
                    continue;
                }
                let mut reply = json!({ "type": "invocationresult",
                    "invocation_id": message["invocation_id"], "function_id": function });
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
            projects_file_path: projects_file.to_str().unwrap().to_string(),
            autocompact: global,
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
            push,
            server,
            _dir: dir,
        };
        // The clients: one observer of each event.
        for (id, function) in [
            ("clients-compaction", ON_COMPACTION_FN),
            ("clients-turn-completed", ON_TURN_COMPLETED_FN),
        ] {
            stack.store().observers.insert(id.into(), function.into());
            stack
                .push
                .send(host::observer_added(
                    id,
                    function,
                    "test::client",
                    json!({}),
                    json!({}),
                ))
                .unwrap();
            let iii = stack.deps.iii.clone();
            wait_until("the clients observe", || iii.is_observed(function)).await;
        }
        stack
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap()
    }

    /// A copy of the store, for assertions with an await in between: a
    /// guard held across it would block the mock engine.
    fn copy(&self) -> Store {
        self.store.lock().unwrap().clone()
    }

    /// Run one `harness::turn` step of `session_id`'s turn.
    async fn run(&self, session_id: &str, step: u64) {
        let turn = self.store().get("harness_turn", session_id);
        let payload = serde_json::from_value(json!({ "session_id": session_id,
            "turn_id": turn["turn_id"], "step": step, "depth": turn["depth"] }))
        .unwrap();
        let _ = tokio::time::timeout(
            Duration::from_secs(20),
            harness::turn_loop::run_step(&self.deps, payload),
        )
        .await
        .expect("the step finishes");
    }

    /// Answer every held call, and stop holding.
    fn release(&self) {
        let held = {
            let mut store = self.store();
            store.hold = None;
            std::mem::take(&mut store.held)
        };
        for (invocation_id, function, data) in held {
            let result = self.store().answer(&function, &data);
            let mut reply = json!({ "type": "invocationresult",
                "invocation_id": invocation_id, "function_id": function });
            match result {
                Ok(value) => reply["result"] = value,
                Err(error) => reply["error"] = json!({ "code": "test_error", "message": error }),
            }
            self.push.send(reply).unwrap();
        }
    }

    async fn held(&self, function: &str) {
        let store = self.store.clone();
        wait_until(&format!("{function} is called"), || {
            store
                .lock()
                .unwrap()
                .held
                .iter()
                .any(|(_, f, _)| f == function)
        })
        .await;
    }

    /// Wait until the clients heard `count` `phase: completed` compactions.
    async fn completions(&self, count: usize) -> Vec<Value> {
        let store = self.store.clone();
        let done = move || {
            store
                .lock()
                .unwrap()
                .events_of(ON_COMPACTION_FN)
                .into_iter()
                .filter(|event| event["phase"] == "completed")
                .collect::<Vec<_>>()
        };
        let probe = done.clone();
        wait_until("the compaction completes", move || probe().len() >= count).await;
        done()
    }

    /// Wait until the clients heard the turn complete.
    async fn turn_completed(&self) -> Value {
        let store = self.store.clone();
        wait_until("the turn completes", || {
            !store
                .lock()
                .unwrap()
                .events_of(ON_TURN_COMPLETED_FN)
                .is_empty()
        })
        .await;
        self.store().events_of(ON_TURN_COMPLETED_FN)[0].clone()
    }
}

async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn global(tokens: Option<u64>) -> AutocompactConfig {
    AutocompactConfig {
        mode: if tokens.is_some() {
            AutocompactMode::Tokens
        } else {
            AutocompactMode::Off
        },
        tokens,
        instructions: Some(INSTRUCTIONS.into()),
    }
}

fn projects() -> Value {
    json!([
        { "id": ACME, "name": "acme-web", "color": "blue", "folders": ["/w/acme"],
          "account": { "mode": "global" }, "autocompact": { "mode": "tokens", "tokens": 160_000 },
          "created_at_ms": 1, "updated_at_ms": 1 },
        { "id": "prj_mobile", "name": "mobile", "color": "teal", "folders": ["/w/mobile"],
          "account": { "mode": "global" }, "autocompact": { "mode": "off" },
          "created_at_ms": 1, "updated_at_ms": 1 },
    ])
}

fn acme_rule() -> Value {
    json!({ "scope": "project", "project_id": ACME, "project_name": "acme-web" })
}

/// A generate step's snapshot: 16k of prompt and tools, `total` in all,
/// the step's `output`, against the model's `usable`.
fn snapshot(session_id: &str, total: u64, output: u64, usable: u64) -> Value {
    let messages = total - 16_000;
    json!({
        "session_id": session_id, "turn_id": TURN, "step": 3, "model": MODEL,
        "provider": "claude-code", "estimator": "heuristic",
        "usable": usable, "effective_max_output_tokens": 64_000,
        "total": total, "free": usable.saturating_sub(total),
        "categories": {
            "system_prompt": 12_000, "skills": 0, "tools": 4_000,
            "messages": { "user": messages / 2, "assistant": messages - messages / 2,
                          "function_result": 0, "custom": 0 },
            "overhead": 0, "hook_guidance": 0,
        },
        "compacted": false,
        "usage": { "input": total, "output": output },
        "autocompact": { "setting_tokens": null, "effective_tokens": null, "cap_tokens": null,
                         "rule": { "scope": "global" } },
        "timestamp": 1_000,
    })
}

/// A turn that ran out of steps (its `max_turns`): its next step ends it
/// through `finalize_completed`. It resolved autocompact off when it started.
fn ended_turn(session_id: &str, snapshot: Value) -> Value {
    json!({
        "turn_id": TURN, "session_id": session_id, "status": "running", "step": 3,
        "turn_count": 4, "depth": 0,
        "options": {
            "model": MODEL, "provider": "claude-code", "max_turns": 4,
            "provider_options": { "claude-code": { "account": "work" } },
        },
        "account": { "id": "work", "source": "session" },
        "autocompact": { "setting_tokens": null, "rule": { "scope": "global" } },
        "context_snapshot": snapshot,
        "created_at": 1, "updated_at": 1,
    })
}

fn user(id: &str, text: &str) -> Value {
    json!({ "entry_id": id, "message": {
        "role": "user", "content": [{ "type": "text", "text": text }], "timestamp": 1 } })
}

fn reply(id: &str, text: &str) -> Value {
    json!({ "entry_id": id, "message": {
        "role": "assistant", "content": [{ "type": "text", "text": text }],
        "stop_reason": "end", "model": MODEL, "provider": "claude-code", "timestamp": 2 } })
}

fn transcript() -> Vec<Value> {
    vec![
        user("u1", "Fix the build."),
        reply("a1", "Looking at the CI log."),
        user("u2", "And the flaky test."),
        reply(&format!("e_{TURN}_3_assistant"), "Both are fixed."),
    ]
}

fn session(session_id: &str, title: &str, metadata: Value) -> Value {
    json!({ "session_id": session_id, "title": title, "status": "working", "metadata": metadata })
}

fn ok_compact() -> Value {
    json!({ "status": "ok", "summary": "## Goal\n- fix the build", "tail_start_index": 2,
            "tokens_before": 120_000, "tokens_after": 9_000, "used_prior_summary": false })
}

/// The conversation in project acme-web (160k) past its threshold at the end
/// of a turn on Claude Sonnet (916k usable): 150k + 12k of output.
fn ended_world(total: u64, output: u64, usable: u64) -> Store {
    let snap = snapshot(S, total, output, usable);
    let mut store = Store {
        compact_reply: ok_compact(),
        count_tokens: COUNTED,
        ..Store::default()
    };
    store.sessions.insert(
        S.into(),
        session(S, "Fix the build", json!({ "project_id": ACME })),
    );
    store.messages.insert(S.into(), transcript());
    store.put("harness_turn", S, ended_turn(S, snap.clone()));
    store.put("harness_context", S, snap);
    store
}

fn turn_record(store: &Store, session_id: &str) -> Value {
    store.get("harness_turn", session_id)
}

/// D158: the end of a turn past `effective_tokens` compacts with
/// `trigger: "auto"`, on the turn's account, with the global instructions;
/// the setting is resolved again (a change made during the turn applies);
/// the refreshed snapshot rides `on-turn-completed`. The step's billed input
/// decides, and is the `context_before`, even where the snapshot's `total`
/// is still context-manager's estimate.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_end_of_a_turn_compacts_past_its_threshold() {
    let mut world = ended_world(150_000, 12_000, 916_000);
    // Estimated 147k + 12k out is under 160k; billed 2k + 148k cached + 12k
    // out is past it.
    let mut snap = snapshot(S, 147_000, 12_000, 916_000);
    snap["usage"] = json!({ "input": 2_000, "cache_read": 148_000, "output": 12_000 });
    world.put("harness_turn", S, ended_turn(S, snap.clone()));
    world.put("harness_context", S, snap);
    let stack = Stack::new(world, projects(), global(None)).await;
    stack.run(S, 3).await;
    let done = stack.completions(1).await;
    let turn_completed = stack.turn_completed().await;
    let store = stack.copy();

    // The summary runs on the conversation's account, with the global
    // instructions (D161).
    let compact = &store.sent("context::compact")[0];
    assert_eq!(compact["model"]["id"], MODEL);
    assert_eq!(
        compact["options"]["provider_options"],
        json!({ "claude-code": { "account": "work" } })
    );
    assert_eq!(compact["options"]["instructions"], INSTRUCTIONS);

    let started = store
        .custom(S, "notice", Some("compacting"))
        .expect("a start notice");
    assert_eq!(started["data"]["message"], "Auto-compacting at 160k…");
    assert_eq!(started["data"]["trigger"], "auto");
    assert_eq!(started["data"]["threshold_tokens"], 160_000);
    assert_eq!(started["data"]["rule"], acme_rule());

    let record = store
        .custom(S, "compaction", None)
        .expect("a compaction record")["data"]
        .clone();
    assert_eq!(record["trigger"], "auto");
    assert_eq!(record["source"], "auto");
    assert_eq!(record["threshold_tokens"], 160_000);
    assert_eq!(record["rule"], acme_rule());
    assert_eq!(record["context_before"], 162_000);
    assert_eq!(record["context_after"], AFTER);
    assert_eq!(record["tokens_before"], 120_000);
    assert_eq!(record["tail_start_entry_id"], "u2");
    let compaction_id = record["compaction_id"].as_str().unwrap().to_string();

    // Every client: started, then completed with contract C's fields.
    let phases: Vec<Value> = store
        .events_of(ON_COMPACTION_FN)
        .iter()
        .map(|event| event["phase"].clone())
        .collect();
    assert_eq!(phases, [json!("started"), json!("completed")]);
    let completed = &done[0];
    assert_eq!(completed["compaction_id"], compaction_id);
    assert_eq!(completed["status"], "ok");
    assert_eq!(completed["trigger"], "auto");
    assert_eq!(completed["source"], "auto");
    assert_eq!(completed["threshold_tokens"], 160_000);
    assert_eq!(completed["rule"], acme_rule());
    assert_eq!(completed["context_before"], 162_000);
    assert_eq!(completed["context_after"], AFTER);
    assert_eq!(
        completed["message"],
        "Auto-compacted at 160k · 162k → 34k tokens · acme-web rule"
    );

    // The meters drop with the completion event: the refreshed snapshot,
    // which keeps the step's autocompact numbers.
    assert_eq!(turn_completed["status"], "completed");
    let context = &turn_completed["context"];
    assert_eq!(context["total"], AFTER);
    assert_eq!(context["compacted"], true);
    assert_eq!(
        context["autocompact"],
        snapshot(S, 150_000, 0, 916_000)["autocompact"],
        "after_compaction keeps it"
    );
    let order: Vec<&str> = store
        .events
        .iter()
        .map(|(function, _)| function.as_str())
        .collect();
    assert_eq!(
        order,
        [ON_COMPACTION_FN, ON_COMPACTION_FN, ON_TURN_COMPLETED_FN],
        "the compaction completes before the turn"
    );

    let turn = turn_record(&store, S);
    assert_eq!(turn["status"], "completed");
    assert!(turn.get("compacting").is_none(), "{turn}");
    assert!(store.get(autocompact::BACKOFF_SCOPE, S).is_null());
    assert!(store.unexpected.is_empty(), "{:?}", store.unexpected);

    // The engine's log, with the title at write time.
    let log = autocompact::log(&stack.deps, LogRequest::default())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&log).unwrap(),
        json!([{
            "session_id": S, "title": "Fix the build", "at_ms": log[0].at_ms,
            "trigger": "auto", "threshold_tokens": 160_000,
            "context_before": 162_000, "context_after": AFTER, "rule": acme_rule(),
        }])
    );
}

/// No compaction at the end of a turn: below the threshold, a setting above
/// the model's `usable` (null `effective_tokens`), off, a sub-agent, a
/// back-off that still holds, and a snapshot whose estimated `total` is past
/// the threshold while the step's billed input is not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_end_of_a_turn_leaves_the_conversation_alone_otherwise() {
    type Prepare = Box<dyn Fn(&mut Store)>;
    let cases: [(&str, Prepare); 6] = [
        (
            "below its threshold",
            Box::new(|store: &mut Store| {
                let snap = snapshot(S, 140_000, 5_000, 916_000);
                store.put("harness_turn", S, ended_turn(S, snap));
            }),
        ),
        (
            "160k on a model with 116k usable",
            Box::new(|store: &mut Store| {
                let snap = snapshot(S, 110_000, 5_000, 116_000);
                store.put("harness_turn", S, ended_turn(S, snap));
            }),
        ),
        (
            "off for its project",
            Box::new(|store: &mut Store| {
                store.sessions.insert(
                    S.into(),
                    session(S, "Fix the build", json!({ "project_id": "prj_mobile" })),
                );
            }),
        ),
        (
            "a sub-agent",
            Box::new(|store: &mut Store| {
                let mut turn = turn_record(store, S);
                turn["depth"] = json!(1);
                turn["parent"] =
                    json!({ "session_id": "s-parent", "turn_id": "t-p", "function_call_id": "c1" });
                store.put("harness_turn", S, turn);
            }),
        ),
        (
            "a back-off that holds",
            Box::new(|store: &mut Store| {
                // 162k < 155k + 10 % of 160k.
                store.put(
                    autocompact::BACKOFF_SCOPE,
                    S,
                    json!({ "at_tokens": 155_000, "reason": "failed", "at_ms": 1 }),
                );
            }),
        ),
        (
            "an estimate past it, the billed input under it",
            Box::new(|store: &mut Store| {
                // A first turn: `exactify` has not applied the billed numbers
                // yet, and the estimate runs high (161,461 + 1,200 ≥ 160k);
                // the step billed 7,622 + 150,000 cached + 1,200 out = 158,822.
                let mut snap = snapshot(S, 161_461, 1_200, 916_000);
                snap["usage"] = json!({ "input": 7_622, "cache_read": 150_000,
                                        "cache_write": 0, "output": 1_200 });
                store.put("harness_turn", S, ended_turn(S, snap.clone()));
                store.put("harness_context", S, snap);
            }),
        ),
    ];
    for (case, prepare) in cases {
        let mut store = ended_world(150_000, 12_000, 916_000);
        prepare(&mut store);
        let stack = Stack::new(store, projects(), global(None)).await;
        stack.run(S, 3).await;
        stack.turn_completed().await;
        let store = stack.store();
        assert!(store.sent("context::compact").is_empty(), "{case}");
        assert!(store.events_of(ON_COMPACTION_FN).is_empty(), "{case}");
        assert_eq!(turn_record(&store, S)["status"], "completed", "{case}");
    }
}

/// A send while the end of a turn compacts waits in the queue and runs after
/// the compaction; `side-chat::open` takes the compacting turn's reply as
/// final, and the side chat's record is not compacting.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_message_sent_while_it_compacts_waits_and_runs_after() {
    let mut store = ended_world(150_000, 12_000, 916_000);
    store.hold = Some("context::compact");
    let stack = Arc::new(Stack::new(store, projects(), global(None)).await);
    let running = tokio::spawn({
        let stack = stack.clone();
        async move { stack.run(S, 3).await }
    });
    stack.held("context::compact").await;
    {
        let store = stack.store();
        let turn = turn_record(&store, S);
        assert_eq!(turn["status"], "running");
        assert!(
            turn["compacting"]
                .as_str()
                .is_some_and(|id| id.starts_with("cmp_")),
            "{turn}"
        );
    }

    // The reply is final: a side chat forks at it.
    let _ = harness::side_chat::open(
        &stack.deps,
        serde_json::from_value(json!({ "session_id": S })).unwrap(),
    )
    .await;
    assert_eq!(
        stack.store().sent("session::fork")[0]["entry_id"],
        format!("e_{TURN}_3_assistant")
    );
    {
        // The side chat's record is a finished turn: the compaction stays the
        // conversation's.
        let store = stack.store();
        let seeded = turn_record(&store, "s-fork");
        assert_eq!(seeded["status"], "completed", "{seeded}");
        assert!(seeded.get("compacting").is_none(), "{seeded}");
    }

    let sent = harness::functions::send::handle(
        &stack.deps,
        serde_json::from_value(json!({ "session_id": S, "message": "next question" })).unwrap(),
    )
    .await
    .expect("the send is accepted");
    assert_eq!(sent.queued, Some(true), "it waits for the compaction");

    stack.release();
    running.await.unwrap();
    stack.turn_completed().await;
    let store = stack.store();
    let entries = store.messages[S].clone();
    let position = |found: &dyn Fn(&Value) -> bool| {
        entries
            .iter()
            .position(found)
            .unwrap_or_else(|| panic!("missing in {entries:#?}"))
    };
    let record = position(&|entry| entry["custom"]["custom_type"] == "compaction");
    let next = position(&|entry| {
        entry["message"]["role"] == "user"
            && entry["message"]["content"][0]["text"] == "next question"
    });
    assert!(record < next, "the queued message lands after the summary");
    // The next turn is seeded and enqueued after the compaction.
    let next_turn = turn_record(&store, S);
    assert_ne!(next_turn["turn_id"], TURN);
    assert_eq!(next_turn["status"], "running");
    let compact_at = store
        .calls
        .iter()
        .position(|(f, _, _)| f == "context::compact")
        .unwrap();
    let enqueued = store
        .calls
        .iter()
        .rposition(|(f, data, action)| {
            f == "harness::turn"
                && action["type"] == "enqueue"
                && data["turn_id"] == next_turn["turn_id"]
        })
        .expect("the next turn is enqueued");
    assert!(compact_at < enqueued);
}

/// A side chat follows its conversation's setting (not its own frozen copy),
/// and `side-chat::summarize` waits while it compacts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_side_chat_follows_its_conversation_and_summarize_waits_for_it() {
    let snap = snapshot(SIDE, 100_000, 5_000, 116_000);
    let mut store = Store {
        compact_reply: ok_compact(),
        count_tokens: COUNTED,
        hold: Some("context::compact"),
        ..Store::default()
    };
    store.sessions.insert(
        S.into(),
        session(
            S,
            "Fix the build",
            json!({ "autocompact": { "mode": "tokens", "tokens": 100_000 } }),
        ),
    );
    store.sessions.insert(
        SIDE.into(),
        session(
            SIDE,
            "Side chat",
            json!({ "side_chat_of": S, "autocompact": { "mode": "off" } }),
        ),
    );
    store.messages.insert(SIDE.into(), transcript());
    store.put("harness_turn", SIDE, ended_turn(SIDE, snap.clone()));
    store.put("harness_context", SIDE, snap);
    let stack = Arc::new(Stack::new(store, projects(), global(None)).await);
    let running = tokio::spawn({
        let stack = stack.clone();
        async move { stack.run(SIDE, 3).await }
    });
    stack.held("context::compact").await;
    let refused = harness::side_chat::summarize(
        &stack.deps,
        serde_json::from_value(json!({ "session_id": SIDE })).unwrap(),
    )
    .await
    .expect_err("busy while it compacts");
    assert_eq!(refused.rejected_code().as_deref(), Some("side_chat_busy"));
    assert!(
        refused
            .to_string()
            .contains("Wait for the side chat to finish compacting"),
        "{refused}"
    );
    stack.release();
    running.await.unwrap();
    let done = stack.completions(1).await;
    assert_eq!(done[0]["rule"], json!({ "scope": "session" }));
    assert_eq!(done[0]["threshold_tokens"], 100_000);
    assert_eq!(
        done[0]["message"],
        "Auto-compacted at 100k · 105k → 34k tokens · this conversation's rule"
    );
}

/// D173: a Stop during the end-of-turn compaction abandons the summary (no
/// record), the completed phase says `cancelled`, and the turn completes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_abandons_the_summary_and_the_turn_still_completes() {
    let mut store = ended_world(150_000, 12_000, 916_000);
    store.hold = Some("context::compact");
    let stack = Arc::new(Stack::new(store, projects(), global(None)).await);
    let running = tokio::spawn({
        let stack = stack.clone();
        async move { stack.run(S, 3).await }
    });
    stack.held("context::compact").await;
    let stopping = tokio::spawn({
        let stack = stack.clone();
        async move {
            harness::functions::stop::handle(
                &stack.deps,
                serde_json::from_value(json!({ "session_id": S })).unwrap(),
            )
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(10), running)
        .await
        .expect("the stop ends the wait for the summary")
        .unwrap();
    let stopped = stopping.await.unwrap().expect("the stop is accepted");
    assert!(stopped.stopping);
    let done = stack.completions(1).await;
    stack.turn_completed().await;
    stack.release();
    let store = stack.store();
    assert_eq!(done[0]["status"], "cancelled");
    assert_eq!(done[0]["trigger"], "auto");
    assert!(store.custom(S, "compaction", None).is_none(), "no record");
    let notice = store
        .custom(S, "notice", Some("compaction_cancelled"))
        .expect("the start notice is closed");
    assert_eq!(
        notice["data"]["message"],
        "Auto-compact stopped. The conversation goes on and compacts again after a later turn."
    );
    assert_eq!(turn_record(&store, S)["status"], "completed");
    // No back-off: a later turn's end compacts at the threshold again (the
    // next one may find context-manager still finishing this summary).
    assert!(store.get(autocompact::BACKOFF_SCOPE, S).is_null());
}

/// A Stop that lands while the turn only compacts stops the compaction only:
/// the reply is final, and a message queued during the summary gets its turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_during_the_summary_leaves_the_queued_message_its_turn() {
    let mut store = ended_world(150_000, 12_000, 916_000);
    store.hold = Some("context::compact");
    let stack = Arc::new(Stack::new(store, projects(), global(None)).await);
    let running = tokio::spawn({
        let stack = stack.clone();
        async move { stack.run(S, 3).await }
    });
    stack.held("context::compact").await;
    let sent = harness::functions::send::handle(
        &stack.deps,
        serde_json::from_value(json!({ "session_id": S, "message": "next question" })).unwrap(),
    )
    .await
    .expect("the send is accepted");
    assert_eq!(sent.queued, Some(true));
    let stopped = harness::functions::stop::handle(
        &stack.deps,
        serde_json::from_value(json!({ "session_id": S })).unwrap(),
    )
    .await
    .expect("the stop is accepted");
    assert!(stopped.stopping, "the stop stopped the compaction");
    tokio::time::timeout(Duration::from_secs(10), running)
        .await
        .expect("the stop ends the wait for the summary")
        .unwrap();
    let done = stack.completions(1).await;
    assert_eq!(done[0]["status"], "cancelled");
    stack.release();

    let store = stack.copy();
    let next_turn = turn_record(&store, S);
    assert_ne!(
        next_turn["turn_id"], TURN,
        "the queued message has its turn"
    );
    assert_eq!(next_turn["status"], "running");
    assert!(
        next_turn.get("abort").is_none_or(|abort| abort == false),
        "{next_turn}"
    );
    assert!(
        store.calls.iter().any(|(f, data, action)| {
            f == "harness::turn"
                && action["type"] == "enqueue"
                && data["turn_id"] == next_turn["turn_id"]
        }),
        "the next turn is enqueued"
    );
    assert!(
        store.messages[S].iter().any(|entry| {
            entry["message"]["role"] == "user"
                && entry["message"]["content"][0]["text"] == "next question"
        }),
        "the queued message reached the transcript"
    );
    assert!(store.custom(S, "compaction", None).is_none(), "no record");
}

/// A Stop whose read lands after the abandoned compaction's turn completed
/// with a queued message (before its reseed, or when the reseed failed)
/// leaves that continuation alone; without the abandoned compaction, a
/// finished turn's continuation is still taken back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_late_stop_keeps_the_continuation_of_a_stopped_compaction() {
    for (stopped_compaction, keeps) in [(true, true), (false, false)] {
        let mut store = ended_world(150_000, 12_000, 916_000);
        let mut turn = turn_record(&store, S);
        turn["status"] = json!("completed");
        turn["continues"] = json!(true);
        if stopped_compaction {
            turn["compaction_stopped"] = json!(true);
        }
        store.put("harness_turn", S, turn);
        let stack = Stack::new(store, projects(), global(None)).await;
        let stopped = harness::functions::stop::handle(
            &stack.deps,
            serde_json::from_value(json!({ "session_id": S })).unwrap(),
        )
        .await;
        let store = stack.copy();
        let turn = turn_record(&store, S);
        if keeps {
            assert!(stopped.expect("the stop is accepted").stopping);
            assert_eq!(turn["continues"], true, "{turn}");
            assert!(
                turn.get("abort").is_none_or(|abort| abort == false),
                "{turn}"
            );
        } else {
            assert!(
                turn.get("continues")
                    .is_none_or(|continues| continues == false),
                "taken back: {turn}"
            );
            assert_eq!(turn["abort"], true, "{turn}");
        }
    }
}

/// D162, D166: a failure says why and backs off; a usage limit names the
/// account and starts no back-off; a gain under 10 % backs off until the
/// context grew by another 10 % of the threshold.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failures_and_small_gains_back_off_and_a_usage_limit_does_not() {
    // A failure.
    let mut store = ended_world(150_000, 12_000, 916_000);
    store.compact_reply = json!({ "status": "overflow", "failure": {
        "kind": "rate_limited", "message": "Too many requests." } });
    let stack = Stack::new(store, projects(), global(None)).await;
    stack.run(S, 3).await;
    let done = stack.completions(1).await;
    {
        let store = stack.store();
        let message = "Auto-compact failed: Too many requests. The conversation goes on and compacts at the model's limit.";
        assert_eq!(done[0]["status"], "failed");
        assert_eq!(done[0]["message"], message);
        assert_eq!(done[0]["failure"]["kind"], "rate_limited");
        let notice = store
            .custom(S, "notice", Some("compaction_failed"))
            .unwrap();
        assert_eq!(notice["data"]["message"], message);
        assert_eq!(notice["data"]["failure"]["kind"], "rate_limited");
        let backoff = store.get(autocompact::BACKOFF_SCOPE, S);
        assert_eq!(backoff["at_tokens"], 162_000);
        assert_eq!(backoff["reason"], "failed");
    }
    drop(stack);

    // A usage limit: the account's name from tmux, no clock time, no back-off.
    let mut store = ended_world(150_000, 12_000, 916_000);
    store.compact_reply = json!({ "status": "overflow", "failure": {
        "kind": "usage_limit", "message": "This Claude Code account reached its usage limit.",
        "error_details": { "usage_limit": true, "window": "five_hour",
                           "resets_at_ms": 1_700_000_000_000_u64 } } });
    store.tmux = Some(json!({ "available": true, "global": "work", "accounts": [
        { "id": "work", "name": "Personal" } ] }));
    let stack = Stack::new(store, projects(), global(None)).await;
    stack.run(S, 3).await;
    let done = stack.completions(1).await;
    {
        let store = stack.store();
        assert_eq!(
            done[0]["message"],
            "Auto-compact failed: Personal reached its usage limit. The conversation goes on and compacts again after its next turn."
        );
        assert_eq!(
            done[0]["failure"]["error_details"]["resets_at_ms"],
            1_700_000_000_000_u64
        );
        assert!(store.get(autocompact::BACKOFF_SCOPE, S).is_null());
    }
    drop(stack);

    // A gain under 10 %: 162k → 151k.
    let mut store = ended_world(150_000, 12_000, 916_000);
    store.count_tokens = 135_000;
    let stack = Stack::new(store, projects(), global(None)).await;
    stack.run(S, 3).await;
    let done = stack.completions(1).await;
    assert_eq!(done[0]["context_after"], 151_000);
    let backoff = stack.store().get(autocompact::BACKOFF_SCOPE, S);
    assert_eq!(backoff["at_tokens"], 162_000);
    assert_eq!(backoff["reason"], "small_gain");
    // The next turn at 170k waits (< 162k + 16k); at 178k it compacts again.
    for (total, compacts) in [(165_000, 1), (173_000, 2)] {
        {
            let mut store = stack.store();
            let snap = snapshot(S, total, 5_000, 916_000);
            store.put("harness_turn", S, ended_turn(S, snap.clone()));
            store.put("harness_context", S, snap);
        }
        stack.run(S, 3).await;
        let store = stack.store();
        assert_eq!(
            store.sent("context::compact").len(),
            compacts,
            "at {total} + 5k"
        );
        assert_eq!(turn_record(&store, S)["status"], "completed");
    }
}

/// A turn's first step that inherits a project setting: the budget, the
/// instructions and the account go to `context::assemble` (D159-D161).
fn generate_world(assemble_reply: Value) -> Store {
    let mut store = Store {
        assemble_reply,
        count_tokens: COUNTED,
        ..Store::default()
    };
    store.sessions.insert(
        S.into(),
        session(S, "Fix the build", json!({ "project_id": ACME })),
    );
    store.messages.insert(S.into(), transcript()[..3].to_vec());
    store.put(
        "harness_turn",
        S,
        json!({
            "turn_id": TURN, "session_id": S, "status": "running", "step": 0,
            "turn_count": 0, "depth": 0,
            "options": {
                "model": MODEL, "provider": "claude-code", "max_turns": 8,
                "provider_options": { "claude-code": { "account": "work" } },
            },
            "account": { "id": "work", "source": "session" },
            "created_at": 1, "updated_at": 1,
        }),
    );
    store
}

fn compacted_assembly(echo: Option<Value>) -> Value {
    let mut reply = json!({
        "token_count": 30_000, "usable": 916_000, "effective_max_output_tokens": 64_000,
        "applied": { "initial_token_count": 181_000, "compacted": true, "summary": "## Goal",
                     "tail_start_index": 1, "tokens_before": 150_000,
                     "summarized_head_tokens": 150_000 },
    });
    if let Some(echo) = echo {
        reply["compaction_budget"] = echo;
        reply["compaction"] = json!({ "trigger": "budget", "status": "compacted" });
    }
    reply
}

/// D159, D160, contract C: an inline compaction at the budget is recorded as
/// `inline` with its id, threshold, rule and context sizes, emits only its
/// completed phase, and joins the log; the snapshot carries the numbers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_inline_compaction_at_the_budget_is_recorded_and_emitted() {
    let stack = Stack::new(
        generate_world(compacted_assembly(Some(json!(180_000)))),
        projects(),
        global(None),
    )
    .await;
    stack.run(S, 0).await;
    let done = stack.completions(1).await;
    let store = stack.copy();
    let assemble = &store.sent("context::assemble")[0];
    assert_eq!(assemble["options"]["compaction_budget"], 180_000);
    assert_eq!(assemble["options"]["compaction_instructions"], INSTRUCTIONS);
    assert_eq!(
        assemble["options"]["provider_options"],
        json!({ "claude-code": { "account": "work" } })
    );

    let compaction_id = format!("cmp_{TURN}_0");
    let record = store.custom(S, "compaction", None).unwrap()["data"].clone();
    assert_eq!(record["compaction_id"], compaction_id);
    assert_eq!(record["trigger"], "inline");
    assert_eq!(record["source"], "auto_inline");
    assert_eq!(
        record["threshold_tokens"], 160_000,
        "effective, not the cap"
    );
    assert_eq!(record["rule"], acme_rule());
    assert_eq!(record["context_before"], 181_000);
    // The assembled count (a recount after stripping pre-compaction thinking
    // replaces context-manager's).
    let after = record["assembled_token_count"].as_u64().unwrap();
    assert_eq!(record["context_after"], after);
    assert!(store.custom(S, "notice", None).is_none(), "no start notice");

    let events = store.events_of(ON_COMPACTION_FN);
    assert_eq!(events.len(), 1, "only the completed phase: {events:?}");
    assert_eq!(
        done[0],
        json!({
            "session_id": S, "compaction_id": compaction_id, "phase": "completed",
            "timestamp": done[0]["timestamp"], "trigger": "inline", "source": "auto_inline",
            "threshold_tokens": 160_000, "rule": acme_rule(), "status": "ok",
            "message": format!(
                "Auto-compacted during the turn at 160k · 181k → {} tokens · acme-web rule",
                autocompact::short_tokens(after)
            ),
            "entry_ids": [format!("e_{TURN}_0_compaction")], "tokens_before": 150_000,
            "tokens_after": after, "context_before": 181_000, "context_after": after,
        })
    );

    let turn = turn_record(&store, S);
    assert_eq!(
        turn["autocompact"],
        json!({ "setting_tokens": 160_000, "rule": acme_rule() })
    );
    assert_eq!(
        turn["context_snapshot"]["autocompact"],
        json!({ "setting_tokens": 160_000, "effective_tokens": 160_000,
                "cap_tokens": 180_000, "rule": acme_rule() })
    );
    let log = autocompact::log(&stack.deps, LogRequest::default())
        .await
        .unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(
        serde_json::to_value(log[0].trigger).unwrap(),
        json!("inline")
    );
}

/// A context-manager older than 1.3.0 echoes no budget: the turn runs at
/// the model's limit, and its compaction is a `limit` one, not logged.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_older_context_manager_compacts_at_the_models_limit() {
    let stack = Stack::new(
        generate_world(compacted_assembly(None)),
        projects(),
        global(None),
    )
    .await;
    stack.run(S, 0).await;
    let done = stack.completions(1).await;
    let store = stack.copy();
    // The budget went out; the older context-manager dropped it.
    assert_eq!(
        store.sent("context::assemble")[0]["options"]["compaction_budget"],
        180_000
    );
    let record = store.custom(S, "compaction", None).unwrap()["data"].clone();
    assert_eq!(record["trigger"], "limit");
    assert_eq!(record["source"], "limit");
    assert!(record.get("threshold_tokens").is_none());
    assert!(record.get("rule").is_none());
    assert_eq!(done[0]["trigger"], "limit");
    let after = record["context_after"].as_u64().unwrap();
    assert_eq!(
        done[0]["message"],
        format!(
            "Compacted at the model's limit · 181k → {} tokens",
            autocompact::short_tokens(after)
        )
    );
    assert!(
        autocompact::log(&stack.deps, LogRequest::default())
            .await
            .unwrap()
            .is_empty(),
        "the log keeps auto and inline only"
    );
}

/// D160 and the clarified contract: a failed or busy inline attempt writes
/// no notice, starts no back-off, and drops the budget for the rest of the
/// turn. So does one at the model's own limit (`trigger: "usable"`): the
/// context is then past the budget too, and every later step would call the
/// summariser again on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_inline_attempt_drops_the_budget_for_the_rest_of_the_turn() {
    for (trigger, usable, status) in [
        ("budget", 916_000, "failed"),
        ("budget", 916_000, "busy"),
        ("usable", 175_000, "failed"),
        ("usable", 175_000, "busy"),
    ] {
        let status = format!("{trigger} {status}");
        let reply = json!({
            "token_count": 170_000, "usable": usable, "effective_max_output_tokens": 64_000,
            "applied": { "initial_token_count": 181_000, "compacted": false },
            "compaction_budget": usable.min(180_000),
            "compaction": { "trigger": trigger, "status": status.split(' ').nth(1),
                            "failure": { "kind": "timeout", "message": "slow" } },
        });
        let stack = Stack::new(generate_world(reply), projects(), global(None)).await;
        stack.run(S, 0).await;
        // The step is delivered again (its provider call never went out).
        stack.run(S, 0).await;
        let store = stack.store();
        let assembled = store.sent("context::assemble");
        assert_eq!(assembled[0]["options"]["compaction_budget"], 180_000);
        assert!(
            assembled
                .last()
                .unwrap()
                .get("options")
                .and_then(|options| options.get("compaction_budget"))
                .is_none(),
            "{status}: no budget after the attempt"
        );
        assert_eq!(turn_record(&store, S)["autocompact"]["paused"], true);
        assert!(store.custom(S, "notice", None).is_none(), "{status}");
        assert!(store.events_of(ON_COMPACTION_FN).is_empty(), "{status}");
        assert!(store.get(autocompact::BACKOFF_SCOPE, S).is_null());
    }
}

/// A side chat reads its conversation's metadata as it is: one with no
/// `autocompact` (empty metadata included) inherits, through its project
/// and the global. Only a conversation that is gone leaves the side chat its
/// frozen copy.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_side_chat_falls_back_to_its_copy_only_when_its_conversation_is_gone() {
    let cases = [
        (Some(json!({})), json!({ "scope": "global" }), Some(300_000)),
        (
            Some(json!({ "project_id": ACME })),
            acme_rule(),
            Some(160_000),
        ),
        (None, json!({ "scope": "session" }), Some(600_000)),
    ];
    for (parent, rule, tokens) in cases {
        let mut store = Store::default();
        if let Some(metadata) = &parent {
            store
                .sessions
                .insert(S.into(), session(S, "Fix the build", metadata.clone()));
        }
        // The side chat's copy, frozen when it opened.
        store.sessions.insert(
            SIDE.into(),
            session(
                SIDE,
                "Side chat",
                json!({ "side_chat_of": S, "project_id": ACME,
                        "autocompact": { "mode": "tokens", "tokens": 600_000 } }),
            ),
        );
        let stack = Stack::new(store, projects(), global(Some(300_000))).await;
        let record =
            serde_json::from_value(ended_turn(SIDE, snapshot(SIDE, 100_000, 5_000, 916_000)))
                .unwrap();
        let session = stack.deps.session().await;
        let resolution = autocompact::resolve(&stack.deps, &session, &record).await;
        assert_eq!(
            serde_json::to_value(&resolution.rule).unwrap(),
            rule,
            "{parent:?}"
        );
        assert_eq!(resolution.setting_tokens, tokens, "{parent:?}");
    }
}

/// A sub-agent copies only `project_id`: it follows its project (and the
/// cap applies inside its turns), never its parent's conversation value.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sub_agent_follows_its_project_not_its_parents_conversation() {
    let mut store = generate_world(compacted_assembly(Some(json!(180_000))));
    store.sessions.insert(
        S.into(),
        session(
            S,
            "Parent",
            json!({ "project_id": ACME, "autocompact": { "mode": "tokens", "tokens": 600_000 } }),
        ),
    );
    store.sessions.insert(
        CHILD.into(),
        session(CHILD, "Child", json!({ "project_id": ACME })),
    );
    store
        .messages
        .insert(CHILD.into(), transcript()[..1].to_vec());
    let mut turn = turn_record(&store, S);
    turn["session_id"] = json!(CHILD);
    turn["depth"] = json!(1);
    turn["parent"] = json!({ "session_id": S, "turn_id": "t-p", "function_call_id": "c1" });
    store.put("harness_turn", CHILD, turn);
    store.put(
        "harness_turn",
        S,
        json!({ "turn_id": "t-p", "session_id": S, "status": "awaiting_functions", "step": 1,
                "turn_count": 1, "depth": 0, "options": { "model": MODEL, "max_turns": 8 },
                "account": { "id": "work", "source": "session" },
                "created_at": 1, "updated_at": 1 }),
    );
    store.tmux = Some(json!({ "available": true, "global": "work", "accounts": [
        { "id": "work", "name": "Personal" } ] }));
    let stack = Stack::new(store, projects(), global(None)).await;
    stack.run(CHILD, 0).await;
    let store = stack.store();
    let assemble = &store.sent("context::assemble")[0];
    assert_eq!(assemble["options"]["compaction_budget"], 180_000);
    assert_eq!(
        turn_record(&store, CHILD)["autocompact"]["rule"],
        acme_rule()
    );
}

/// D161: `/compact` and `harness::compact` resolve the conversation's
/// account at the call; a side chat uses its lock's; another provider's
/// model gets none.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_command_compaction_runs_on_the_conversations_account() {
    let mut store = Store {
        compact_reply: ok_compact(),
        count_tokens: COUNTED,
        tmux: Some(json!({ "available": true, "global": "work", "accounts": [
            { "id": "work", "name": "Work" }, { "id": "personal", "name": "Personal" } ] })),
        ..Store::default()
    };
    store.sessions.insert(
        S.into(),
        session(S, "Fix the build", json!({ "claude_account": "personal" })),
    );
    store.sessions.insert(
        SIDE.into(),
        session(SIDE, "Side chat", json!({ "side_chat_of": S })),
    );
    // `/compact` sent from a client, on a conversation of its own.
    store.sessions.insert(
        "s-command".into(),
        session(
            "s-command",
            "Command",
            json!({ "claude_account": "personal" }),
        ),
    );
    for id in [S, SIDE, "s-command"] {
        store.messages.insert(id.into(), transcript());
    }
    store.put(
        "harness_side_chat",
        SIDE,
        json!({ "session_id": SIDE, "parent_session_id": S, "model": MODEL,
                "provider": "claude-code",
                "account": { "id": "work", "source": "session" },
                "created_at_ms": 1, "source": "open" }),
    );
    let stack = Stack::new(store, projects(), global(None)).await;
    let compact = |session_id: &str, model: &str| {
        serde_json::from_value(json!({ "session_id": session_id, "model": model,
            "provider": model.split('/').next() }))
        .unwrap()
    };
    harness::functions::compact::handle(&stack.deps, compact(S, MODEL))
        .await
        .unwrap();
    harness::functions::compact::handle(&stack.deps, compact(SIDE, MODEL))
        .await
        .unwrap();
    harness::functions::compact::handle(&stack.deps, compact(S, "anthropic/claude-x"))
        .await
        .unwrap();
    // `/compact` through `harness::send` runs in the background.
    let sent = harness::functions::send::handle(
        &stack.deps,
        serde_json::from_value(json!({ "session_id": "s-command", "message": "/compact",
            "model": MODEL, "provider": "claude-code" }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(sent.compaction.is_some());
    stack.completions(4).await;
    let store = stack.copy();
    let options: Vec<Value> = store
        .sent("context::compact")
        .iter()
        .map(|request| request["options"]["provider_options"].clone())
        .collect();
    assert_eq!(
        options,
        [
            json!({ "claude-code": { "account": "personal" } }),
            json!({ "claude-code": { "account": "work" } }),
            Value::Null,
            json!({ "claude-code": { "account": "personal" } }),
        ]
    );
    // `/compact` keeps its own words and records `trigger: "command"`.
    let record = store.custom(S, "compaction", None).unwrap()["data"].clone();
    assert_eq!(record["trigger"], "command");
    assert_eq!(record["source"], "command");
    assert!(record.get("threshold_tokens").is_none());
    assert!(
        autocompact::log(&stack.deps, LogRequest::default())
            .await
            .unwrap()
            .is_empty()
    );
}

fn log_outcome(session_id: &str, n: u64) -> harness::compaction::Outcome {
    harness::compaction::inline_outcome(
        session_id,
        &format!("cmp_{n}"),
        harness::compaction::Trigger::Inline,
        Some(160_000),
        None,
        None,
        Some(170_000 + n),
        Some(30_000),
        Vec::new(),
    )
}

/// D166: one document, a ring of 50 updated with compare-and-swap (a write
/// that raced another harness keeps both), newest first; deleting a session
/// drops its entries and its back-off.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_log_is_a_ring_of_fifty_and_a_delete_drops_a_session() {
    let mut store = Store::default();
    for id in [S, SIDE] {
        store
            .sessions
            .insert(id.into(), session(id, &format!("Title {id}"), json!({})));
    }
    store.put(
        autocompact::BACKOFF_SCOPE,
        SIDE,
        json!({ "at_tokens": 1, "reason": "failed", "at_ms": 1 }),
    );
    store.log_interference = Some(json!({
        "session_id": "s-other", "title": "Another harness", "at_ms": 1, "trigger": "auto",
        "threshold_tokens": 100_000, "context_before": 101_000, "context_after": 20_000,
        "rule": { "scope": "global" } }));
    let stack = Stack::new(store, projects(), global(None)).await;
    let session = stack.deps.session().await;
    autocompact::log_compaction(&stack.deps, &session, &log_outcome(S, 0)).await;
    let log = autocompact::log(&stack.deps, LogRequest::default())
        .await
        .unwrap();
    let ids: Vec<&str> = log.iter().map(|entry| entry.session_id.as_str()).collect();
    assert_eq!(ids, [S, "s-other"], "the raced write kept both");
    assert_eq!(log[0].title, format!("Title {S}"));

    for n in 1..=60 {
        let id = if n % 2 == 0 { S } else { SIDE };
        autocompact::log_compaction(&stack.deps, &session, &log_outcome(id, n)).await;
    }
    let all = autocompact::log(&stack.deps, LogRequest { limit: Some(500) })
        .await
        .unwrap();
    assert_eq!(all.len(), 50);
    assert_eq!(all[0].context_before, Some(170_060), "newest first");
    assert_eq!(all[49].context_before, Some(170_011));
    assert_eq!(
        autocompact::log(&stack.deps, LogRequest::default())
            .await
            .unwrap()
            .len(),
        20
    );

    autocompact::forget_session(&stack.deps, SIDE)
        .await
        .unwrap();
    let left = autocompact::log(&stack.deps, LogRequest { limit: Some(50) })
        .await
        .unwrap();
    assert_eq!(left.len(), 25);
    assert!(left.iter().all(|entry| entry.session_id == S));
    assert!(
        stack
            .store()
            .get(autocompact::BACKOFF_SCOPE, SIDE)
            .is_null()
    );
}

/// D165: the models that ran a turn here in the last 7 days, newest first,
/// named from the router catalog (an unlisted model keeps its id and has no
/// window).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn models_lists_what_ran_this_week_with_the_catalog() {
    let mut store = Store::default();
    store.catalog.insert(
        MODEL.into(),
        json!({ "id": MODEL, "provider": "claude-code", "display_name": "Claude Opus 5.5",
                "context_window": 200_000, "max_output_tokens": 64_000 }),
    );
    store.put(
        autocompact::MODELS_SCOPE,
        "old",
        json!({ "provider": "anthropic", "model": "claude-old", "usable": 100_000,
                "last_used_ms": 1 }),
    );
    let stack = Stack::new(store, projects(), global(None)).await;
    autocompact::note_model(&stack.deps, Some("claude-code"), MODEL, 116_000).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    autocompact::note_model(&stack.deps, None, "local/tiny", 20_000).await;
    let models = autocompact::models(&stack.deps, ModelsRequest {})
        .await
        .unwrap();
    let models = serde_json::to_value(&models).unwrap();
    assert_eq!(models.as_array().unwrap().len(), 2, "{models}");
    assert_eq!(models[0]["model"], "local/tiny");
    assert_eq!(models[0]["provider"], "local");
    assert_eq!(models[0]["name"], "local/tiny");
    assert_eq!(models[0]["context_window"], Value::Null);
    assert_eq!(models[1]["model"], MODEL);
    assert_eq!(models[1]["name"], "Claude Opus 5.5");
    assert_eq!(models[1]["context_window"], 200_000);
    assert_eq!(models[1]["usable"], 116_000);
}
