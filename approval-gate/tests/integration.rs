//! Engine-backed integration suite — the worker's production surface
//! registered against a real iii engine (`iii-worker-manager +
//! iii-pubsub + configuration + iii-state`), with the not-yet-built
//! siblings (`policy::check_permissions`, `harness::function::resolve`,
//! `session::get`) registered as in-process fakes.
//!
//! **Self-skips** when no engine is available (llm-router pattern):
//! set `III_ENGINE_BIN=/path/to/iii` or have `iii` on PATH.

use std::io::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iii_sdk::{
    register_worker, InitOptions, RegisterFunction, RegisterTriggerInput, TriggerRequest, III,
};
use serde_json::{json, Value};

use approval_gate::bus::IiiBus;
use approval_gate::config::WorkerConfig;
use approval_gate::events::{self, Emitter};
use approval_gate::functions::{self, Deps};
use approval_gate::gate_config::{self, replace, shared_defaults, SharedDefaults};

// ── engine bootstrap (llm-router/tests/integration.rs pattern) ─────────────

struct Engine {
    url: String,
    child: std::process::Child,
    dir: std::path::PathBuf,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn engine_bin() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("III_ENGINE_BIN") {
        return Some(p.into());
    }
    let on_path = std::process::Command::new("iii")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    on_path.then(|| "iii".into())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

async fn spawn_engine() -> Option<Engine> {
    let bin = engine_bin()?;
    let port = free_port();
    let dir = std::env::temp_dir().join(format!("approval-gate-it-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");

    let config = format!(
        r#"workers:
  - name: iii-worker-manager
    config:
      port: {port}
  - name: iii-pubsub
    config:
      adapter:
        name: local
  - name: configuration
    config:
      adapter:
        name: fs
        config:
          directory: {dir}/configuration
      ttl_seconds: 0
  - name: iii-state
    config:
      adapter:
        name: kv
        config:
          file_path: {dir}/state_store.db
          store_method: file_based
"#,
        port = port,
        dir = dir.display(),
    );
    let config_path = dir.join("config.yaml");
    std::fs::File::create(&config_path)
        .and_then(|mut f| f.write_all(config.as_bytes()))
        .expect("write config");

    let child = std::process::Command::new(&bin)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&config_path)
        .current_dir(&dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn engine");

    let url = format!("ws://127.0.0.1:{port}");
    let probe = register_worker(&url, InitOptions::default());
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let ready = probe
            .trigger(TriggerRequest {
                function_id: "engine::workers::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(1000),
            })
            .await
            .is_ok();
        if ready {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "engine did not become ready in 15s"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    probe.shutdown();

    Some(Engine { url, child, dir })
}

macro_rules! engine_or_skip {
    () => {
        match spawn_engine().await {
            Some(e) => e,
            None => {
                eprintln!("skipping: no iii engine (set III_ENGINE_BIN or put `iii` on PATH)");
                return;
            }
        }
    };
}

// ── worker bootstrap + fakes ────────────────────────────────────────────────

type CallLog = Arc<Mutex<Vec<Value>>>;

struct Stack {
    iii: Arc<III>,
    defaults: SharedDefaults,
    /// Requests received by the fake harness::function::resolve.
    harness_calls: CallLog,
    /// approval::pending_created deliveries.
    created: CallLog,
    /// approval::pending_resolved deliveries.
    resolved: CallLog,
}

fn log_push(log: &CallLog, value: Value) {
    log.lock().unwrap_or_else(|p| p.into_inner()).push(value);
}

fn log_snapshot(log: &CallLog) -> Vec<Value> {
    log.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Register the production surface + fakes + event recorders on one
/// worker connection against the engine.
async fn boot_stack(engine: &Engine, policy_decision: Value) -> Stack {
    let iii = Arc::new(register_worker(&engine.url, InitOptions::default()));

    // Production: trigger types first, then functions (main.rs order).
    let sets = events::register_trigger_types(&iii);
    let bus = Arc::new(IiiBus::new(iii.clone()));
    let sink = Arc::new(Emitter::new(sets, bus.clone()));
    let defaults = shared_defaults();
    let deps = Arc::new(Deps {
        bus: bus.clone(),
        sink,
        defaults: defaults.clone(),
        cfg: Arc::new(WorkerConfig::default()),
    });
    functions::register_all(&iii, &deps);

    // Fake siblings.
    iii.register_function(
        "policy::check_permissions",
        RegisterFunction::new_async(move |_req: Value| {
            let decision = policy_decision.clone();
            async move { Ok::<_, iii_sdk::IIIError>(decision) }
        }),
    );
    let harness_calls: CallLog = Arc::default();
    {
        let log = harness_calls.clone();
        iii.register_function(
            "harness::function::resolve",
            RegisterFunction::new_async(move |req: Value| {
                let log = log.clone();
                async move {
                    log_push(&log, req);
                    Ok::<_, iii_sdk::IIIError>(json!({ "resolved": true, "turn_resumed": true }))
                }
            }),
        );
    }
    iii.register_function(
        "session::get",
        RegisterFunction::new_async(move |_req: Value| async move {
            Ok::<_, iii_sdk::IIIError>(json!({ "meta": {
                "session_id": "s_1",
                "title": "Integration session",
                "description": "",
                "metadata": { "owner": "u_1" }
            }}))
        }),
    );

    // Event recorders, bound through the engine like a real notification
    // worker would bind.
    let created: CallLog = Arc::default();
    {
        let log = created.clone();
        iii.register_function(
            "recorder::on_created",
            RegisterFunction::new_async(move |req: Value| {
                let log = log.clone();
                async move {
                    log_push(&log, req);
                    Ok::<_, iii_sdk::IIIError>(Value::Null)
                }
            }),
        );
    }
    let resolved: CallLog = Arc::default();
    {
        let log = resolved.clone();
        iii.register_function(
            "recorder::on_resolved",
            RegisterFunction::new_async(move |req: Value| {
                let log = log.clone();
                async move {
                    log_push(&log, req);
                    Ok::<_, iii_sdk::IIIError>(Value::Null)
                }
            }),
        );
    }
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: events::PENDING_CREATED.to_string(),
        function_id: "recorder::on_created".to_string(),
        config: json!({}),
        metadata: None,
    })
    .expect("bind pending_created");
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: events::PENDING_RESOLVED.to_string(),
        function_id: "recorder::on_resolved".to_string(),
        config: json!({}),
        metadata: None,
    })
    .expect("bind pending_resolved");

    // Reactive config reload binding + entry registration + initial read.
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "configuration".to_string(),
        function_id: "approval::on_config_change".to_string(),
        config: json!({
            "configuration_id": gate_config::ENTRY_ID,
            "event_types": ["configuration:registered", "configuration:updated"],
        }),
        metadata: None,
    })
    .expect("bind configuration trigger");
    gate_config::register_entry(bus.as_ref())
        .await
        .expect("register configuration entry");
    replace(&defaults, gate_config::read_defaults(bus.as_ref()).await);

    Stack {
        iii,
        defaults,
        harness_calls,
        created,
        resolved,
    }
}

async fn call(iii: &III, function_id: &str, payload: Value) -> Result<Value, iii_sdk::IIIError> {
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
}

fn hook_input(session_id: &str, call_id: &str, function_id: &str) -> Value {
    json!({
        "point": "pre_dispatch",
        "session_id": session_id,
        "turn_id": "t_1",
        "step": 1,
        "depth": 0,
        "call": {
            "id": call_id,
            "function_id": function_id,
            "arguments": { "cmd": "ls", "api_key": "sk_live_secret" }
        }
    })
}

/// Poll until `predicate` is true or the deadline passes (fire-and-forget
/// deliveries are async).
async fn wait_for(deadline_ms: u64, mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(deadline_ms);
    loop {
        if predicate() {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// ── scenarios ───────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn hold_writes_record_emits_once_and_is_idempotent() {
    let engine = engine_or_skip!();
    let stack = boot_stack(&engine, json!({ "decision": "needs_approval" })).await;
    let iii = &stack.iii;

    let out = call(
        iii,
        "approval::gate",
        hook_input("s_1", "c_1", "shell::run"),
    )
    .await
    .expect("gate call");
    assert_eq!(out["decision"], json!("hold"));
    assert_eq!(out["pending_timeout_ms"], json!(1_800_000));

    // The record is in the real state worker, redacted, with session
    // context from the fake session-manager.
    let record = call(
        iii,
        "state::get",
        json!({ "scope": "approval_pending", "key": "s_1/c_1" }),
    )
    .await
    .expect("state get");
    assert_eq!(record["function_id"], json!("shell::run"));
    assert_eq!(record["arguments_excerpt"]["api_key"], json!("<redacted>"));
    assert_eq!(record["session_title"], json!("Integration session"));
    assert_eq!(record["session_metadata"]["owner"], json!("u_1"));

    // Inbox read surfaces it.
    let listed = call(iii, "approval::list_pending", json!({}))
        .await
        .unwrap();
    assert_eq!(listed["pending"].as_array().unwrap().len(), 1);

    // Duplicate hold (redelivered step): still hold, no second emission.
    let again = call(
        iii,
        "approval::gate",
        hook_input("s_1", "c_1", "shell::run"),
    )
    .await
    .expect("gate call");
    assert_eq!(again["decision"], json!("hold"));

    assert!(
        wait_for(3_000, || !log_snapshot(&stack.created).is_empty()).await,
        "pending_created should reach the bound recorder"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let created = log_snapshot(&stack.created);
    assert_eq!(created.len(), 1, "exactly one pending_created: {created:?}");
    assert_eq!(created[0]["status"], json!("pending"));
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_allow_releases_and_deny_delivers_through_the_fake_harness() {
    let engine = engine_or_skip!();
    let stack = boot_stack(&engine, json!({ "decision": "needs_approval" })).await;
    let iii = &stack.iii;

    // Hold two calls.
    for cid in ["c_allow", "c_deny"] {
        let out = call(iii, "approval::gate", hook_input("s_1", cid, "shell::run"))
            .await
            .unwrap();
        assert_eq!(out["decision"], json!("hold"));
    }

    // Allow → action execute, no content.
    let res = call(
        iii,
        "approval::resolve",
        json!({ "session_id": "s_1", "function_call_id": "c_allow", "decision": "allow" }),
    )
    .await
    .unwrap();
    assert_eq!(res["resolved"], json!(true));
    assert_eq!(res["turn_resumed"], json!(true));

    // Deny → action deliver, is_error, envelope in details.
    let res = call(
        iii,
        "approval::resolve",
        json!({
            "session_id": "s_1",
            "function_call_id": "c_deny",
            "decision": "deny",
            "reason": "not on prod"
        }),
    )
    .await
    .unwrap();
    assert_eq!(res["resolved"], json!(true));

    let harness = log_snapshot(&stack.harness_calls);
    assert_eq!(harness.len(), 2);
    assert_eq!(harness[0]["action"], json!("execute"));
    assert_eq!(harness[0]["turn_id"], json!("t_1"));
    assert!(harness[0].get("content").is_none());
    assert_eq!(harness[1]["action"], json!("deliver"));
    assert_eq!(harness[1]["is_error"], json!(true));
    assert_eq!(harness[1]["details"]["denied_by"], json!("user"));
    assert_eq!(harness[1]["content"][0]["text"], json!("not on prod"));

    // Both records gone — and not as tombstones: the real state worker's
    // scope list is empty again (risk #2 verification).
    let listed = call(iii, "approval::list_pending", json!({}))
        .await
        .unwrap();
    assert_eq!(listed["pending"].as_array().unwrap().len(), 0);
    let raw = call(iii, "state::list", json!({ "scope": "approval_pending" }))
        .await
        .unwrap();
    let live: Vec<&Value> = raw
        .as_array()
        .map(|a| a.iter().filter(|v| !v.is_null()).collect())
        .unwrap_or_default();
    assert!(live.is_empty(), "no live or tombstone records: {raw}");

    // Exactly one pending_resolved per record.
    assert!(
        wait_for(3_000, || log_snapshot(&stack.resolved).len() >= 2).await,
        "pending_resolved should reach the bound recorder"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let resolved = log_snapshot(&stack.resolved);
    assert_eq!(resolved.len(), 2, "{resolved:?}");
    let outcomes: Vec<&str> = resolved
        .iter()
        .filter_map(|e| e["outcome"].as_str())
        .collect();
    assert!(outcomes.contains(&"allow") && outcomes.contains(&"deny"));

    // Duplicate resolve: benign no-op.
    let dup = call(
        iii,
        "approval::resolve",
        json!({ "session_id": "s_1", "function_call_id": "c_allow", "decision": "allow" }),
    )
    .await
    .unwrap();
    assert_eq!(dup["resolved"], json!(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn sweep_expires_records_and_emits_timeout_exactly_once() {
    let engine = engine_or_skip!();
    let stack = boot_stack(&engine, json!({ "decision": "needs_approval" })).await;
    let iii = &stack.iii;

    // Seed an already-expired record straight into the real state worker.
    call(
        iii,
        "state::set",
        json!({ "scope": "approval_pending", "key": "s_9/c_9", "value": {
            "session_id": "s_9",
            "turn_id": "t_9",
            "function_call_id": "c_9",
            "function_id": "shell::run",
            "arguments_excerpt": {},
            "pending_at": 100,
            "expires_at": 200,
            "depth": 0,
        }}),
    )
    .await
    .unwrap();

    let swept = call(iii, "approval::sweep", json!({})).await.unwrap();
    assert_eq!(swept["swept"], json!(1));

    let harness = log_snapshot(&stack.harness_calls);
    assert_eq!(harness.len(), 1);
    assert_eq!(harness[0]["action"], json!("deliver"));
    assert_eq!(harness[0]["is_error"], json!(true));
    assert_eq!(harness[0]["details"]["status"], json!("timeout"));

    // Second sweep: nothing left, no double resolution, no double event.
    let swept = call(iii, "approval::sweep", json!({})).await.unwrap();
    assert_eq!(swept["swept"], json!(0));

    assert!(
        wait_for(3_000, || !log_snapshot(&stack.resolved).is_empty()).await,
        "timeout event should reach the recorder"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let resolved = log_snapshot(&stack.resolved);
    assert_eq!(resolved.len(), 1, "{resolved:?}");
    assert_eq!(resolved[0]["outcome"], json!("timeout"));
}

#[tokio::test(flavor = "multi_thread")]
async fn configuration_set_reloads_defaults_reactively() {
    let engine = engine_or_skip!();
    let stack = boot_stack(&engine, json!({ "decision": "needs_approval" })).await;
    let iii = &stack.iii;

    // Manual default: the gate holds.
    let out = call(
        iii,
        "approval::gate",
        hook_input("s_2", "c_1", "shell::run"),
    )
    .await
    .unwrap();
    assert_eq!(out["decision"], json!("hold"));

    // Operator flips the deployment default to full.
    call(
        iii,
        "configuration::set",
        json!({ "id": "approval-gate", "value": { "default_mode": "full" } }),
    )
    .await
    .expect("configuration set");

    // The configuration trigger swaps the in-memory defaults.
    assert!(
        wait_for(5_000, || {
            gate_config::snapshot(&stack.defaults).default_mode
                == approval_gate::types::PermissionMode::Full
        })
        .await,
        "defaults should reload reactively"
    );

    // A session with no stored settings now runs full → continue.
    let out = call(
        iii,
        "approval::gate",
        hook_input("s_3", "c_2", "shell::run"),
    )
    .await
    .unwrap();
    assert_eq!(out["decision"], json!("continue"));
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_are_lazily_seeded_against_real_state() {
    let engine = engine_or_skip!();
    let stack = boot_stack(&engine, json!({ "decision": "needs_approval" })).await;
    let iii = &stack.iii;

    // Reads never write.
    let before = call(
        iii,
        "approval::get_settings",
        json!({ "session_id": "s_lazy" }),
    )
    .await
    .unwrap();
    assert_eq!(before["source"], json!("defaults"));
    let raw = call(
        iii,
        "state::get",
        json!({ "scope": "approval_settings", "key": "s_lazy" }),
    )
    .await
    .unwrap();
    assert!(raw.is_null(), "read materialized a record: {raw}");

    // First mutation materializes.
    let res = call(
        iii,
        "approval::approve_always",
        json!({ "session_id": "s_lazy", "function_id": "shell::run" }),
    )
    .await
    .unwrap();
    assert_eq!(
        res["settings"]["approved_always"][0]["function_id"],
        json!("shell::run")
    );
    let after = call(
        iii,
        "approval::get_settings",
        json!({ "session_id": "s_lazy" }),
    )
    .await
    .unwrap();
    assert_eq!(after["source"], json!("stored"));

    // approve_always holds in manual mode: the gate now allows.
    let out = call(
        iii,
        "approval::gate",
        hook_input("s_lazy", "c_1", "shell::run"),
    )
    .await
    .unwrap();
    assert_eq!(out["decision"], json!("continue"));

    // clear_settings reverts to defaults.
    let cleared = call(
        iii,
        "approval::clear_settings",
        json!({ "session_id": "s_lazy" }),
    )
    .await
    .unwrap();
    assert_eq!(cleared["cleared"], json!(true));
    let reverted = call(
        iii,
        "approval::get_settings",
        json!({ "session_id": "s_lazy" }),
    )
    .await
    .unwrap();
    assert_eq!(reverted["source"], json!("defaults"));
}

#[tokio::test(flavor = "multi_thread")]
async fn human_only_targets_are_denied_through_the_full_stack() {
    let engine = engine_or_skip!();
    let stack = boot_stack(&engine, json!({ "decision": "allow" })).await;
    let iii = &stack.iii;

    let out = call(
        iii,
        "approval::gate",
        hook_input("s_1", "c_1", "approval::set_mode"),
    )
    .await
    .unwrap();
    assert_eq!(out["decision"], json!("deny"));
    assert!(out["reason"]
        .as_str()
        .unwrap()
        .contains("human_only_function"));
}
