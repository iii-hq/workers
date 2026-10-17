//! `/compact` through `harness::send` and `harness::compact`, end to end over
//! an isolated engine with stub session-manager / state / context-manager
//! workers and a `harness::compaction` subscriber standing in for a second
//! device. No model runs. Skips when no `iii` engine binary is available.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use harness::config::WorkerConfig;
use harness::configuration::ConfigCell;
use harness::deps::Deps;
use harness::discovery;
use harness::events::TurnEvents;
use harness::functions::compact::{self, CompactRequest};
use harness::functions::send::{self, MessageInput, SendRequest};
use harness::functions::status::{self, StatusRequest};
use harness::hooks::HookRegistry;
use harness::skills;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::{Notify, RwLock};

const SESSION: &str = "s-parent";

struct Engine {
    url: String,
    child: std::process::Child,
    _dir: tempfile::TempDir,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn worker_options(name: &str) -> InitOptions {
    InitOptions {
        metadata: Some(WorkerMetadata {
            name: format!("harness-compact-test-{name}-{}", std::process::id()),
            ..WorkerMetadata::default()
        }),
        ..InitOptions::default()
    }
}

async fn isolated_engine() -> Option<Engine> {
    let binary = std::env::var_os("III_ENGINE_BIN")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::process::Command::new("iii")
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .ok()
                .filter(|status| status.success())
                .map(|_| std::path::PathBuf::from("iii"))
        })?;
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()?
        .port();
    let dir = tempfile::tempdir().ok()?;
    let config = dir.path().join("config.yaml");
    std::fs::write(
        &config,
        format!("workers:\n  - name: iii-worker-manager\n    config:\n      port: {port}\n"),
    )
    .ok()?;
    let mut child = std::process::Command::new(binary)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&config)
        .current_dir(dir.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let url = format!("ws://127.0.0.1:{port}");
    let probe = register_worker(&url, worker_options("probe"));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if probe
            .trigger(TriggerRequest {
                function_id: "engine::workers::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(500),
            })
            .await
            .is_ok()
        {
            probe.shutdown();
            return Some(Engine {
                url,
                child,
                _dir: dir,
            });
        }
        if Instant::now() >= deadline {
            probe.shutdown();
            let _ = child.kill();
            let _ = child.wait();
            if std::env::var_os("III_ENGINE_BIN").is_some() {
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                panic!("iii engine did not become ready: {stderr}");
            }
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// What the stub workers serve and what they were asked.
#[derive(Default)]
struct World {
    transcript: Vec<Value>,
    /// `session::append` payloads, in order.
    appended: Vec<Value>,
    statuses: Vec<(String, Option<String>)>,
    /// `state::*` store, keyed `<scope>/<key>`.
    state: BTreeMap<String, Value>,
    compact_requests: Vec<Value>,
    compact_reply: Value,
    /// `harness::compaction` events the subscriber received.
    events: Vec<Value>,
}

type Shared = Arc<Mutex<World>>;

/// The transcript as `session::messages` serves it: the seeded entries and
/// every custom entry appended since.
fn served_messages(world: &World) -> Value {
    let mut messages = world.transcript.clone();
    for payload in &world.appended {
        messages.push(json!({
            "entry_id": payload["entry_id"],
            "custom": payload["custom"],
        }));
    }
    json!({ "messages": messages, "next_cursor": null })
}

fn entry(id: &str, message: Value) -> Value {
    json!({ "entry_id": id, "message": message })
}

fn user(id: &str, text: &str) -> Value {
    entry(
        id,
        json!({ "role": "user", "content": [{ "type": "text", "text": text }], "timestamp": 1 }),
    )
}

fn reply(id: &str, text: &str) -> Value {
    entry(
        id,
        json!({
            "role": "assistant", "content": [{ "type": "text", "text": text }],
            "stop_reason": "end", "model": "claude-x", "provider": "anthropic", "timestamp": 2
        }),
    )
}

fn call(id: &str, call_id: &str) -> Value {
    entry(
        id,
        json!({
            "role": "assistant",
            "content": [{ "type": "function_call", "id": call_id, "function_id": "harness::spawn", "arguments": {} }],
            "stop_reason": "function_call", "model": "claude-x", "provider": "anthropic", "timestamp": 3
        }),
    )
}

fn turn_record(turn_id: &str, status: &str) -> Value {
    json!({
        "turn_id": turn_id, "session_id": SESSION, "status": status, "step": 3,
        "turn_count": 3, "depth": 0,
        "options": { "model": "claude-x", "provider": "anthropic", "max_turns": 16 },
        "created_at": 1, "updated_at": 1
    })
}

fn snapshot() -> Value {
    json!({
        "session_id": SESSION, "turn_id": "t-done", "step": 3, "model": "claude-x",
        "provider": "anthropic", "estimator": "heuristic",
        "usable": 800_000, "effective_max_output_tokens": 64_000,
        "total": 480_000, "free": 320_000,
        "categories": {
            "system_prompt": 12_000, "skills": 0, "tools": 4_000,
            "messages": { "user": 60_000, "assistant": 104_000, "function_result": 300_000, "custom": 0 },
            "overhead": 0, "hook_guidance": 0
        },
        "compacted": false,
        "usage": { "input": 480_000, "output": 900 },
        "timestamp": 1_000
    })
}

struct Stack {
    deps: Deps,
    world: Shared,
    /// Holds `context::compact` until notified, when set.
    gate: Arc<Mutex<Option<Arc<Notify>>>>,
    _workers: Vec<Arc<IIIClient>>,
    _engine: Engine,
}

impl Stack {
    async fn new(world: World) -> Option<Self> {
        let engine = isolated_engine().await?;
        let world: Shared = Arc::new(Mutex::new(world));
        let gate: Arc<Mutex<Option<Arc<Notify>>>> = Arc::default();
        let stubs = register_worker(&engine.url, worker_options("stubs"));

        let w = world.clone();
        stubs.register_function(
            "session::get",
            RegisterFunction::new_async(move |payload: Value| {
                let _ = &w;
                async move {
                    Ok::<_, iii_sdk::Error>(json!({
                        "meta": { "session_id": payload["session_id"], "status": "done", "metadata": {} }
                    }))
                }
            }),
        );
        let w = world.clone();
        stubs.register_function(
            "session::messages",
            RegisterFunction::new_async(move |_: Value| {
                let w = w.clone();
                async move { Ok::<_, iii_sdk::Error>(served_messages(&w.lock().unwrap())) }
            }),
        );
        let w = world.clone();
        stubs.register_function(
            "session::append",
            RegisterFunction::new_async(move |payload: Value| {
                let w = w.clone();
                async move {
                    let entry_id = payload["entry_id"].clone();
                    w.lock().unwrap().appended.push(payload);
                    Ok::<_, iii_sdk::Error>(json!({ "entry_id": entry_id }))
                }
            }),
        );
        let w = world.clone();
        stubs.register_function(
            "session::set-status",
            RegisterFunction::new_async(move |payload: Value| {
                let w = w.clone();
                async move {
                    w.lock().unwrap().statuses.push((
                        payload["status"].as_str().unwrap_or_default().to_string(),
                        payload["reason"].as_str().map(str::to_string),
                    ));
                    Ok::<_, iii_sdk::Error>(json!({}))
                }
            }),
        );
        for function_id in ["state::get", "harness::state::get"] {
            let w = world.clone();
            stubs.register_function(
                function_id,
                RegisterFunction::new_async(move |payload: Value| {
                    let w = w.clone();
                    async move {
                        let key = format!(
                            "{}/{}",
                            payload["scope"].as_str().unwrap_or_default(),
                            payload["key"].as_str().unwrap_or_default()
                        );
                        Ok::<_, iii_sdk::Error>(
                            w.lock()
                                .unwrap()
                                .state
                                .get(&key)
                                .cloned()
                                .unwrap_or(Value::Null),
                        )
                    }
                }),
            );
        }
        let w = world.clone();
        stubs.register_function(
            "state::set",
            RegisterFunction::new_async(move |payload: Value| {
                let w = w.clone();
                async move {
                    let key = format!(
                        "{}/{}",
                        payload["scope"].as_str().unwrap_or_default(),
                        payload["key"].as_str().unwrap_or_default()
                    );
                    w.lock()
                        .unwrap()
                        .state
                        .insert(key, payload["value"].clone());
                    Ok::<_, iii_sdk::Error>(json!({}))
                }
            }),
        );
        stubs.register_function(
            "state::list",
            RegisterFunction::new_async(|_: Value| async { Ok::<_, iii_sdk::Error>(json!([])) }),
        );
        let (w, g) = (world.clone(), gate.clone());
        stubs.register_function(
            "context::compact",
            RegisterFunction::new_async(move |payload: Value| {
                let (w, g) = (w.clone(), g.clone());
                async move {
                    let wait = g.lock().unwrap().clone();
                    if let Some(gate) = wait {
                        gate.notified().await;
                    }
                    let mut world = w.lock().unwrap();
                    world.compact_requests.push(payload);
                    Ok::<_, iii_sdk::Error>(world.compact_reply.clone())
                }
            }),
        );
        stubs.register_function(
            "context::count-tokens",
            RegisterFunction::new_async(|payload: Value| async move {
                let messages = payload["messages"].as_array().map_or(0, Vec::len) as u64;
                Ok::<_, iii_sdk::Error>(json!({
                    "tokens": 1_000 + messages * 100,
                    "by_role": { "user": messages * 40, "assistant": messages * 60, "function_result": 0, "custom": 0 }
                }))
            }),
        );
        let w = world.clone();
        stubs.register_function(
            "test::compaction-events",
            RegisterFunction::new_async(move |payload: Value| {
                let w = w.clone();
                async move {
                    w.lock().unwrap().events.push(payload);
                    Ok::<_, iii_sdk::Error>(json!({}))
                }
            }),
        );
        stubs
            .wait_until_registered(Duration::from_secs(5))
            .await
            .expect("stubs registered");

        let iii = Arc::new(register_worker(&engine.url, worker_options("harness")));
        iii.wait_until_registered(Duration::from_secs(5))
            .await
            .expect("harness registered");
        let config: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        let deps = Deps::new(
            iii.clone(),
            config,
            discovery::new_cell(),
            skills::new_cell(),
            TurnEvents::register(&iii),
            HookRegistry::register(&iii),
        );
        // Another device watching the session.
        let listener = register_worker(&engine.url, worker_options("listener"));
        listener
            .register_trigger(RegisterTriggerInput {
                trigger_type: "harness::compaction".into(),
                function_id: "test::compaction-events".into(),
                config: json!({ "session_id": SESSION }),
                metadata: None,
                namespace: None,
                trigger_namespace: None,
            })
            .expect("subscription registered");
        tokio::time::sleep(Duration::from_millis(500)).await;
        Some(Self {
            deps,
            world,
            gate,
            _workers: vec![Arc::new(stubs), Arc::new(listener), iii],
            _engine: engine,
        })
    }

    fn world(&self) -> std::sync::MutexGuard<'_, World> {
        self.world.lock().unwrap()
    }

    async fn send(&self, text: &str, key: &str) -> send::SendResponse {
        send::handle(
            &self.deps,
            SendRequest {
                session_id: Some(SESSION.into()),
                message: MessageInput::Text(text.into()),
                model: Some("claude-x".into()),
                provider: Some("anthropic".into()),
                idempotency_key: Some(key.into()),
                session: None,
                options: None,
            },
        )
        .await
        .expect("send accepted")
    }

    /// Wait until the subscriber heard `count` completion events.
    async fn completions(&self, count: usize) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let done: Vec<Value> = self
                .world()
                .events
                .iter()
                .filter(|e| e["phase"] == "completed")
                .cloned()
                .collect();
            if done.len() >= count {
                return done;
            }
            assert!(
                Instant::now() < deadline,
                "compaction never completed: {:?}",
                self.world().events
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

fn idle_world(compact_reply: Value) -> World {
    World {
        transcript: vec![
            user("u1", "Coordinate the observe spec with sub-agents."),
            reply("a1", "Spawned the executors."),
            user("n1", "[notification] obs04-kan6-exec-e5b1: done"),
            reply("a2", "Reviewer next."),
            user("n2", "[notification] obs04-kan6-review-a9d3: done"),
            reply("a3", "Round 2 queued."),
        ],
        state: BTreeMap::from([
            (
                format!("harness_turn/{SESSION}"),
                turn_record("t-done", "completed"),
            ),
            (format!("harness_context/{SESSION}"), snapshot()),
        ]),
        compact_reply,
        ..World::default()
    }
}

fn ok_reply(tail_start_index: Option<usize>) -> Value {
    json!({
        "status": "ok", "summary": "## Goal\n- observe spec", "tail_start_index": tail_start_index,
        "tokens_before": 471_486, "tokens_after": 9_000, "used_prior_summary": false
    })
}

fn custom_of<'a>(world: &'a World, entry_id: &str) -> &'a Value {
    &world
        .appended
        .iter()
        .find(|payload| payload["entry_id"] == entry_id)
        .unwrap_or_else(|| panic!("no entry {entry_id}: {:?}", world.appended))["custom"]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_compact_send_compacts_on_the_server_and_every_client_hears_about_it() {
    let Some(stack) = Stack::new(idle_world(ok_reply(Some(4)))).await else {
        eprintln!("skipping: iii engine is unavailable");
        return;
    };

    let sent = stack.send("/compact keep the ticket ids", "msg-1").await;
    let started = sent.compaction.clone().expect("a compaction, not a turn");
    assert_eq!(started.entry_id, "e_idem_msg-1");
    assert_eq!(sent.turn_id, "t-done", "no turn started");
    assert!(sent.queued.is_none() && sent.merged.is_none());

    let done = stack.completions(1).await;
    let total = {
        let world = stack.world();

        // The model never saw `/compact`: only custom rows were written.
        assert!(world.appended.iter().all(|p| p.get("message").is_none()));
        // The sender's optimistic row reconciles into the start notice.
        let notice = custom_of(&world, "e_idem_msg-1");
        assert_eq!(notice["custom_type"], "notice");
        assert_eq!(notice["data"]["message"], "compacting the conversation…");

        // Router-resolved limits, the session lease, the user's guidance and the
        // window the next turn would assemble.
        let request = &world.compact_requests[0];
        assert_eq!(
            request["model"],
            json!({ "id": "claude-x", "provider": "anthropic" })
        );
        assert_eq!(request["options"]["lease_key"], SESSION);
        assert_eq!(request["options"]["instructions"], "keep the ticket ids");
        assert_eq!(request["messages"].as_array().unwrap().len(), 6);

        let outcome_entry = format!("e_{}_outcome", started.compaction_id);
        let record = custom_of(&world, &outcome_entry);
        assert_eq!(record["custom_type"], "compaction");
        assert_eq!(record["data"]["tail_start_entry_id"], "n2");
        assert_eq!(record["data"]["summary"], "## Goal\n- observe spec");
        assert_eq!(record["data"]["tokens_before"], 471_486);

        // Every device: working while it ran, back to done after.
        assert_eq!(
            world.statuses,
            vec![
                (
                    "working".to_string(),
                    Some("compacting context".to_string())
                ),
                ("done".to_string(), None),
            ]
        );
        let phases: Vec<&Value> = world.events.iter().map(|e| &e["phase"]).collect();
        assert_eq!(phases, ["started", "completed"]);
        assert_eq!(done[0]["status"], "ok");
        assert_eq!(done[0]["message"], "compacted 471,486 tokens");
        assert_eq!(done[0]["entry_ids"], json!(["e_idem_msg-1", outcome_entry]));

        // The context chip's snapshot now describes the compacted window.
        let written = world.state[&format!("harness_context/{SESSION}")].clone();
        assert_eq!(written["compacted"], true);
        assert!(written.get("usage").is_none());
        let total = written["total"].as_u64().unwrap();
        assert!(
            total < 480_000,
            "snapshot still shows the old window: {total}"
        );
        total
    };

    let status = status::handle(
        &stack.deps,
        StatusRequest {
            session_id: SESSION.into(),
            verbose: true,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        status.context.map(|c| c.total),
        Some(total),
        "harness::status reads the newer snapshot"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_repeated_compact_send_is_deduplicated() {
    let Some(stack) = Stack::new(idle_world(ok_reply(Some(4)))).await else {
        eprintln!("skipping: iii engine is unavailable");
        return;
    };
    stack.send("/compact", "msg-dup").await;
    stack.completions(1).await;
    let again = stack.send("/compact", "msg-dup").await;
    assert_eq!(again.deduplicated, Some(true));
    assert!(again.compaction.is_none());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(stack.world().compact_requests.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failures_reach_every_client_as_a_notice_and_an_event() {
    for (reply, status, message) in [
        (
            json!({ "status": "overflow" }),
            "failed",
            "compaction failed — the summariser could not produce a summary with claude-x; try again, or switch to a model with a larger context window",
        ),
        (
            json!({ "status": "empty" }),
            "empty",
            "nothing to compact yet — the conversation still fits word for word.",
        ),
        (
            json!({ "status": "busy" }),
            "busy",
            "compaction skipped — another compaction of this conversation is in progress; try /compact again when it finishes.",
        ),
    ] {
        let Some(stack) = Stack::new(idle_world(reply)).await else {
            eprintln!("skipping: iii engine is unavailable");
            return;
        };
        let outcome = compact::handle(
            &stack.deps,
            CompactRequest {
                session_id: SESSION.into(),
                model: None,
                provider: None,
                instructions: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(serde_json::to_value(outcome.status).unwrap(), status);
        assert_eq!(outcome.message, message);
        let done = stack.completions(1).await;
        let world = stack.world();
        // Started under its own id (no sender row), ended with a notice.
        let started = format!("e_{}_started", outcome.compaction_id);
        let ended = format!("e_{}_outcome", outcome.compaction_id);
        assert_eq!(outcome.entry_ids, vec![started.clone(), ended.clone()]);
        assert_eq!(custom_of(&world, &started)["data"]["message"], "compacting the conversation…");
        let notice = custom_of(&world, &ended);
        assert_eq!(notice["custom_type"], "notice");
        assert_eq!(notice["data"]["message"], message);
        assert_eq!(done[0]["status"], status);
        // The session's last turn model writes the summary.
        assert_eq!(world.compact_requests[0]["model"]["id"], "claude-x");
        assert!(!world.appended.iter().any(|p| p["custom"]["custom_type"] == "compaction"));
        // A waiting caller shows its own progress: the status line is untouched.
        assert!(world.statuses.is_empty(), "{:?}", world.statuses);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_turns_pending_call_stays_verbatim_and_keeps_its_status_line() {
    let mut world = idle_world(ok_reply(None));
    world.transcript.push(call("e_t-live_0_assistant", "c9"));
    world.state.insert(
        format!("harness_turn/{SESSION}"),
        turn_record("t-live", "awaiting_functions"),
    );
    let Some(stack) = Stack::new(world).await else {
        eprintln!("skipping: iii engine is unavailable");
        return;
    };
    let sent = stack.send("/compact", "msg-live").await;
    let started = sent.compaction.expect("compaction");
    stack.completions(1).await;
    let world = stack.world();
    // The unanswered call is not sent to the summariser…
    assert_eq!(
        world.compact_requests[0]["messages"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    // …and opens the verbatim tail, so its result lands next to it.
    let record = custom_of(&world, &format!("e_{}_outcome", started.compaction_id));
    assert_eq!(
        record["data"]["tail_start_entry_id"],
        "e_t-live_0_assistant"
    );
    // The live turn owns the status line.
    assert!(world.statuses.is_empty(), "{:?}", world.statuses);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_compact_while_one_runs_answers_on_its_own_row() {
    let Some(stack) = Stack::new(idle_world(ok_reply(Some(4)))).await else {
        eprintln!("skipping: iii engine is unavailable");
        return;
    };
    let gate = Arc::new(Notify::new());
    *stack.gate.lock().unwrap() = Some(gate.clone());
    stack.send("/compact", "msg-a").await;
    // The first is now inside the summariser.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !stack.world().events.iter().any(|e| e["phase"] == "started") {
        assert!(Instant::now() < deadline, "first compaction never started");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    stack.send("/compact", "msg-b").await;
    let first_done = stack.completions(1).await;
    assert_eq!(first_done[0]["status"], "busy");
    {
        let world = stack.world();
        let answer = custom_of(&world, "e_idem_msg-b");
        assert_eq!(
            answer["data"]["message"],
            "compaction skipped — this conversation is already being compacted; its result will appear here."
        );
    }
    *stack.gate.lock().unwrap() = None;
    gate.notify_one();
    let all = stack.completions(2).await;
    assert_eq!(all[1]["status"], "ok");
    assert_eq!(stack.world().compact_requests.len(), 1);
}
