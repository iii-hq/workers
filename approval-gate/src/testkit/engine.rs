//! Engine-backed test bootstrap (llm-router / integration.rs pattern).
//! Self-skips when no `iii` binary is on PATH or `III_ENGINE_BIN` is set.

use std::io::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iii_sdk::{
    register_worker, InitOptions, RegisterFunction, RegisterTriggerInput, TriggerRequest, III,
};
use serde_json::{json, Value};
use tokio::sync::OnceCell;

use crate::config::WorkerConfig;
use crate::events::{self, Emitter, PENDING_CREATED, PENDING_RESOLVED};
use crate::functions::{self, Deps};
use crate::gate_config::{self, replace, shared_defaults, SharedDefaults, ENTRY_ID};

pub struct Engine {
    pub url: String,
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

pub fn engine_bin() -> Option<std::path::PathBuf> {
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

pub async fn spawn_engine() -> Option<Engine> {
    let bin = engine_bin()?;
    let port = free_port();
    let dir = std::env::temp_dir().join(format!(
        "approval-gate-it-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&dir).ok()?;

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
        .ok()?;

    let mut child = std::process::Command::new(&bin)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&config_path)
        .current_dir(&dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

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
        if Instant::now() >= deadline {
            probe.shutdown();
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&dir);
            return None;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    probe.shutdown();

    Some(Engine { url, child, dir })
}

static BOOT_LOCK: OnceCell<tokio::sync::Mutex<()>> = OnceCell::const_new();

async fn boot_lock() -> tokio::sync::MutexGuard<'static, ()> {
    BOOT_LOCK
        .get_or_init(|| async { tokio::sync::Mutex::new(()) })
        .await
        .lock()
        .await
}

/// Spawn a fresh isolated engine. Serialized to avoid port races when tests
/// run in parallel; each call gets its own state store.
pub async fn require_engine() -> Option<Engine> {
    let _guard = boot_lock().await;
    match spawn_engine().await {
        Some(engine) => Some(engine),
        None => {
            eprintln!("skipping: no iii engine (set III_ENGINE_BIN or put `iii` on PATH)");
            None
        }
    }
}

pub type CallLog = Arc<Mutex<Vec<Value>>>;

#[derive(Clone)]
pub enum PolicyStub {
    Reply(Value),
    Error(String),
    Absent,
}

impl Default for PolicyStub {
    fn default() -> Self {
        Self::Reply(json!({ "decision": "needs_approval" }))
    }
}

#[derive(Clone, Default)]
pub struct BootOpts {
    pub policy: PolicyStub,
}

impl BootOpts {
    pub fn needs_approval() -> Self {
        Self {
            policy: PolicyStub::Reply(json!({ "decision": "needs_approval" })),
        }
    }

    pub fn allow() -> Self {
        Self {
            policy: PolicyStub::Reply(json!({ "decision": "allow" })),
        }
    }

    pub fn deny() -> Self {
        Self {
            policy: PolicyStub::Reply(json!({ "decision": "deny", "rule_id": "test" })),
        }
    }

    pub fn policy_reply(value: Value) -> Self {
        Self {
            policy: PolicyStub::Reply(value),
        }
    }

    pub fn policy_error(message: impl Into<String>) -> Self {
        Self {
            policy: PolicyStub::Error(message.into()),
        }
    }

    pub fn no_policy() -> Self {
        Self {
            policy: PolicyStub::Absent,
        }
    }
}

pub struct TestStack {
    pub iii: Arc<III>,
    pub deps: Arc<Deps>,
    pub defaults: SharedDefaults,
    /// Requests received by the fake `harness::function::resolve`.
    pub harness_calls: CallLog,
    /// `approval::pending-created` deliveries.
    pub created: CallLog,
    /// `approval::pending-resolved` deliveries.
    pub resolved: CallLog,
}

pub fn log_push(log: &CallLog, value: Value) {
    log.lock().unwrap_or_else(|p| p.into_inner()).push(value);
}

pub fn log_snapshot(log: &CallLog) -> Vec<Value> {
    log.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Register the production surface + fakes + event recorders on one
/// worker connection against the engine.
pub async fn boot(engine: &Engine, opts: BootOpts) -> TestStack {
    let iii = Arc::new(register_worker(&engine.url, InitOptions::default()));

    let sets = events::register_trigger_types(&iii);
    let defaults = shared_defaults();
    let deps = Arc::new(Deps {
        iii: iii.clone(),
        sink: Arc::new(Emitter::new(sets, iii.clone())),
        defaults: defaults.clone(),
        cfg: Arc::new(WorkerConfig::default()),
    });
    functions::register_all(&iii, &deps);

    let policy = opts.policy;
    match policy {
        PolicyStub::Reply(decision) => {
            iii.register_function(
                "policy::check_permissions",
                RegisterFunction::new_async(move |_req: Value| {
                    let decision = decision.clone();
                    async move { Ok::<_, iii_sdk::IIIError>(decision) }
                }),
            );
        }
        PolicyStub::Error(message) => {
            iii.register_function(
                "policy::check_permissions",
                RegisterFunction::new_async(move |_req: Value| {
                    let message = message.clone();
                    async move { Err::<Value, _>(iii_sdk::IIIError::Handler(message)) }
                }),
            );
        }
        PolicyStub::Absent => {}
    }

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
        RegisterFunction::new_async(move |req: Value| async move {
            let session_id = req
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or("s_1");
            Ok::<_, iii_sdk::IIIError>(json!({ "meta": {
                "session_id": session_id,
                "title": "Integration session",
                "description": "",
                "metadata": { "owner": "u_1" }
            }}))
        }),
    );

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
        trigger_type: PENDING_CREATED.to_string(),
        function_id: "recorder::on_created".to_string(),
        config: json!({}),
        metadata: None,
    })
    .expect("bind pending_created");
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: PENDING_RESOLVED.to_string(),
        function_id: "recorder::on_resolved".to_string(),
        config: json!({}),
        metadata: None,
    })
    .expect("bind pending_resolved");

    let _ = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "configuration".to_string(),
        function_id: "approval::on-config-change".to_string(),
        config: json!({
            "configuration_id": ENTRY_ID,
            "event_types": ["configuration:registered", "configuration:updated"],
        }),
        metadata: None,
    });
    if let Err(e) = gate_config::register_entry(&iii).await {
        tracing::debug!(error = %e, "configuration entry register skipped");
    }
    replace(&defaults, gate_config::read_defaults(&iii).await);

    TestStack {
        iii,
        deps,
        defaults,
        harness_calls,
        created,
        resolved,
    }
}

pub async fn call(
    iii: &III,
    function_id: &str,
    payload: Value,
) -> Result<Value, iii_sdk::IIIError> {
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(30_000),
    })
    .await
}

pub async fn state_get(iii: &III, scope: &str, key: &str) -> Value {
    call(iii, "state::get", json!({ "scope": scope, "key": key }))
        .await
        .expect("state::get")
}

pub async fn state_set(iii: &III, scope: &str, key: &str, value: Value) {
    call(
        iii,
        "state::set",
        json!({ "scope": scope, "key": key, "value": value }),
    )
    .await
    .expect("state::set");
}

pub async fn settle() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

pub fn hook_input(session_id: &str, call_id: &str, function_id: &str) -> Value {
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
pub async fn wait_for(deadline_ms: u64, mut predicate: impl FnMut() -> bool) -> bool {
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

/// Run `f` against a freshly booted stack; skip when no engine is available.
pub async fn with_stack<F, Fut>(opts: BootOpts, f: F)
where
    F: FnOnce(TestStack) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let Some(engine) = require_engine().await else {
        return;
    };
    let stack = boot(&engine, opts).await;
    f(stack).await;
    // `engine` drops here — isolated state cleaned up with the temp dir.
}
