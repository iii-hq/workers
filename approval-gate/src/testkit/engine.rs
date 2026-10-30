//! Engine-backed test bootstrap (llm-router / integration.rs pattern).
//!
//! The stack is an engine (`III_ENGINE_BIN`, or `iii` on PATH) with its
//! `configuration` worker, plus the standalone `state` worker of this repo
//! (`III_STATE_BIN`, a built `state/target/debug/state`): the engine hosts no
//! state worker of its own. Tests skip when either binary is unavailable and
//! fail when both are there and the stack does not come up.

use std::io::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iii_sdk::{
    register_worker, CallRequest, Deliver, IIIClient, InitOptions, ObserverHandle,
    RegisterFunction, Source,
};
use serde_json::{json, Value};
use tokio::sync::{OnceCell, RwLock};

use crate::configuration::{self, ConfigCell};
use crate::events::{self, Emitter, ON_PENDING_CREATED_FN, ON_PENDING_RESOLVED_FN};
use crate::functions::{self, Deps};

pub struct Engine {
    pub url: String,
    child: std::process::Child,
    state: Option<std::process::Child>,
    dir: std::path::PathBuf,
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(mut state) = self.state.take() {
            let _ = state.kill();
            let _ = state.wait();
        }
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

/// The standalone `state` worker binary (`III_STATE_BIN`).
pub fn state_bin() -> Option<std::path::PathBuf> {
    std::env::var_os("III_STATE_BIN").map(Into::into)
}

/// Why the engine-backed stack cannot run here, or `None` when it can.
pub fn stack_unavailable() -> Option<&'static str> {
    if engine_bin().is_none() {
        return Some("no iii engine (set III_ENGINE_BIN or put `iii` on PATH)");
    }
    if state_bin().is_none() {
        return Some("no state worker (set III_STATE_BIN to a built `state` binary)");
    }
    None
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

/// Start an isolated engine and the state worker on it. `None` when a binary
/// is unavailable ([`stack_unavailable`] says which); panics when both are
/// there and the stack does not answer within 30 s.
pub async fn spawn_engine() -> Option<Engine> {
    let bin = engine_bin()?;
    let state_bin = state_bin()?;
    let port = free_port();
    let dir = std::env::temp_dir().join(format!(
        "approval-gate-it-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&dir).ok()?;
    // The configuration worker's fs adapter writes one YAML file per id
    // under this directory; create it up front so register/get never race
    // a missing path (approval-gate now treats configuration as a required
    // boot dependency).
    std::fs::create_dir_all(dir.join("configuration")).ok()?;

    let config = format!(
        r#"workers:
  - name: iii-worker-manager
    config:
      port: {port}
  - name: configuration
    config:
      adapter:
        name: fs
        config:
          directory: {dir}/configuration
      ttl_seconds: 0
"#,
        port = port,
        dir = dir.display(),
    );
    let config_path = dir.join("config.yaml");
    std::fs::File::create(&config_path)
        .and_then(|mut f| f.write_all(config.as_bytes()))
        .expect("write the engine config");
    // The state worker's seed: an in-memory kv store, gone with the stack.
    let state_seed = dir.join("state.yaml");
    std::fs::write(
        &state_seed,
        "adapter:\n  name: kv\n  config:\n    store_method: in_memory\n",
    )
    .expect("write the state worker seed");

    let child = std::process::Command::new(&bin)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&config_path)
        .current_dir(&dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn the engine {}: {e}", bin.display()));
    let url = format!("ws://127.0.0.1:{port}");
    let mut engine = Engine {
        url: url.clone(),
        child,
        state: None,
        dir: dir.clone(),
    };

    let probe = register_worker(&url, InitOptions::default());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        // The engine core must be up AND the configuration worker must be
        // serving — approval-gate treats configuration as a required boot
        // dependency, so a test that boots before it is ready would fail
        // spuriously. Probe a dummy id and accept any response (including
        // NOT_FOUND) as "the worker is up".
        let core_ready = probe
            .call(CallRequest {
                function_id: "engine::workers::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(1000),
            })
            .await
            .is_ok();
        let ready = core_ready
            && match probe
                .call(CallRequest {
                    function_id: "configuration::get".into(),
                    payload: json!({ "id": "__readiness_probe__" }),
                    action: None,
                    timeout_ms: Some(1000),
                })
                .await
            {
                Ok(_) => true,
                Err(e) => e.to_string().to_ascii_uppercase().contains("NOT_FOUND"),
            };
        if ready {
            break;
        }
        if Instant::now() >= deadline {
            probe.shutdown();
            panic!("the engine on {url} did not answer within 30 s");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    engine.state = Some(
        std::process::Command::new(&state_bin)
            .arg("--url")
            .arg(&url)
            .arg("--config")
            .arg(&state_seed)
            .current_dir(&dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("spawn the state worker {}: {e}", state_bin.display())),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while probe
        .call(CallRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": "__readiness_probe__", "key": "__readiness_probe__" }),
            action: None,
            timeout_ms: Some(1000),
        })
        .await
        .is_err()
    {
        if Instant::now() >= deadline {
            probe.shutdown();
            panic!("the state worker on {url} did not answer within 30 s");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    probe.shutdown();

    Some(engine)
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
    let engine = spawn_engine().await;
    if engine.is_none() {
        eprintln!("skipping: {}", stack_unavailable().unwrap_or_default());
    }
    engine
}

pub type CallLog = Arc<Mutex<Vec<Value>>>;

/// How the booted stack's inline permission `rules` are set. The gate
/// evaluates `WorkerConfig::rules` directly (no policy worker), so a test
/// drives it by overriding that list in the config snapshot.
#[derive(Clone)]
pub enum RulesOverride {
    /// Keep the shipped built-in rules seeded into the configuration entry.
    Shipped,
    /// No rules — every call falls through to `needs_approval`.
    Empty,
    /// An explicit rule list (string shorthands or rule objects).
    Custom(Vec<Value>),
}

#[derive(Clone)]
pub struct BootOpts {
    pub rules: RulesOverride,
    /// When true, spawn the real `harness` worker and skip the fake
    /// `harness::function::resolve` stub.
    pub real_harness: bool,
    /// Test-only replacement written while the fake function resolver runs,
    /// modeling a post-call hook that parks the same call again.
    pub function_resolve_replacement: Option<Value>,
}

impl Default for BootOpts {
    fn default() -> Self {
        Self::needs_approval()
    }
}

impl BootOpts {
    /// Empty rules: every call needs approval (the default test posture).
    pub fn needs_approval() -> Self {
        Self {
            rules: RulesOverride::Empty,
            real_harness: false,
            function_resolve_replacement: None,
        }
    }

    /// Like [`Self::needs_approval`] but boots the real harness worker.
    pub fn needs_approval_with_harness() -> Self {
        Self {
            rules: RulesOverride::Empty,
            real_harness: true,
            function_resolve_replacement: None,
        }
    }

    /// Allow everything (`["*"]`).
    pub fn allow() -> Self {
        Self {
            rules: RulesOverride::Custom(vec![json!("*")]),
            real_harness: false,
            function_resolve_replacement: None,
        }
    }

    /// Allow exactly one function id / glob.
    pub fn allow_function(function_id: impl Into<String>) -> Self {
        Self {
            rules: RulesOverride::Custom(vec![json!(function_id.into())]),
            real_harness: false,
            function_resolve_replacement: None,
        }
    }

    /// Deny exactly one function id / glob (`!`-prefixed shorthand).
    pub fn deny_function(function_id: impl Into<String>) -> Self {
        Self {
            rules: RulesOverride::Custom(vec![json!(format!("!{}", function_id.into()))]),
            real_harness: false,
            function_resolve_replacement: None,
        }
    }

    /// An explicit rule list.
    pub fn with_rules(rules: Vec<Value>) -> Self {
        Self {
            rules: RulesOverride::Custom(rules),
            real_harness: false,
            function_resolve_replacement: None,
        }
    }

    /// The shipped built-in rules from configuration defaults.
    pub fn shipped_rules() -> Self {
        Self {
            rules: RulesOverride::Shipped,
            real_harness: false,
            function_resolve_replacement: None,
        }
    }

    pub fn replacing_pending_on_resolve(mut self, replacement: Value) -> Self {
        self.function_resolve_replacement = Some(replacement);
        self
    }
}

pub struct TestStack {
    pub iii: Arc<IIIClient>,
    pub deps: Arc<Deps>,
    /// The live config snapshot the handlers read (same cell as
    /// `deps.config`); write to it to drive per-call tuning knobs and
    /// approval defaults in tests.
    pub config: ConfigCell,
    /// Requests received by the fake `harness::function::resolve`.
    pub harness_calls: CallLog,
    /// `approval::on-pending-created` events, delivered as output.
    pub created: CallLog,
    /// `approval::on-pending-resolved` events, delivered as output.
    pub resolved: CallLog,
    harness_child: Option<std::process::Child>,
}

impl Drop for TestStack {
    fn drop(&mut self) {
        if let Some(mut child) = self.harness_child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn_harness_worker(engine_url: &str) -> Option<std::process::Child> {
    let bin = std::env::var("CARGO_BIN_EXE_harness").ok()?;
    std::process::Command::new(bin)
        .arg("--url")
        .arg(engine_url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()
}

async fn wait_for_harness(iii: &IIIClient) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if iii
            .call(CallRequest {
                function_id: "harness::status".into(),
                payload: json!({ "session_id": "__probe__" }),
                action: None,
                timeout_ms: Some(500),
            })
            .await
            .is_ok()
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
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

    // `configuration` is a required boot dependency: register the schema
    // (seeding built-in defaults on first boot) and fetch the authoritative
    // value, exactly as the production binary does.
    configuration::register_config(&iii, None)
        .await
        .expect("register approval-gate configuration schema");
    let mut cfg = configuration::fetch_config(&iii)
        .await
        .expect("fetch approval-gate configuration");
    // Drive the gate's inline permission rules from the boot options. The
    // gate compiles `WorkerConfig::rules` per call, so overriding the list
    // here is all a test needs to choose allow / deny / hold behaviour.
    match &opts.rules {
        RulesOverride::Shipped => {}
        RulesOverride::Empty => cfg.rules = Vec::new(),
        RulesOverride::Custom(rules) => cfg.rules = rules.clone(),
    }
    let config: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg)));

    events::register_functions(&iii);
    let deps = Arc::new(Deps {
        iii: iii.clone(),
        sink: Arc::new(Emitter::new(iii.clone())),
        config: config.clone(),
    });
    functions::register_all(&iii, &deps);

    let harness_calls: CallLog = Arc::default();
    if !opts.real_harness {
        let log = harness_calls.clone();
        let iii_for_resolve = iii.clone();
        let replacement = opts.function_resolve_replacement.clone();
        iii.register_function(
            "harness::function::resolve",
            RegisterFunction::new_async(move |req: Value| {
                let log = log.clone();
                let iii = iii_for_resolve.clone();
                let replacement = replacement.clone();
                async move {
                    log_push(&log, req.clone());
                    let has_replacement = replacement.is_some();
                    if let Some(replacement) = replacement {
                        let session_id = req["session_id"].as_str().unwrap_or("s_1");
                        let call_id = req["function_call_id"].as_str().unwrap_or("c_1");
                        state_set(
                            &iii,
                            crate::pending::PENDING_SCOPE,
                            &format!("{session_id}/{call_id}"),
                            replacement,
                        )
                        .await;
                    }
                    Ok::<_, iii_sdk::errors::Error>(json!({
                        "resolved": true,
                        "turn_resumed": !has_replacement,
                    }))
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
            Ok::<_, iii_sdk::errors::Error>(json!({ "meta": {
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
                    Ok::<_, iii_sdk::errors::Error>(Value::Null)
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
                    Ok::<_, iii_sdk::errors::Error>(Value::Null)
                }
            }),
        );
    }
    // The recorders observe both event functions; wait until each observer
    // is active, so the emitter (which calls only while observed) reaches
    // them from the first test step on.
    for (function_id, recorder) in [
        (ON_PENDING_CREATED_FN, "recorder::on_created"),
        (ON_PENDING_RESOLVED_FN, "recorder::on_resolved"),
    ] {
        let handle = iii.observe(
            Source::function(function_id).deliver(Deliver::Output),
            recorder,
        );
        await_active(&handle, function_id).await;
    }

    let gate_observer = configuration::observe_hook(&iii);

    // The typed, re-fetching config-change handler + its observer of
    // `configuration::on-updated` (mirrors the production boot order: last).
    configuration::observe_config_changes(&iii, config.clone());

    let harness_child = if opts.real_harness {
        let child = spawn_harness_worker(&engine.url).expect("spawn harness worker");
        assert!(
            wait_for_harness(&iii).await,
            "harness worker did not become ready"
        );
        // The gate's observer of `harness::hook::on-pre-call` stays pending
        // until the harness registers its hook function.
        await_active(&gate_observer, configuration::HOOK_ON_PRE_CALL_FN).await;
        Some(child)
    } else {
        None
    };

    TestStack {
        iii,
        deps,
        config,
        harness_calls,
        created,
        resolved,
        harness_child,
    }
}

/// Wait (bounded) until an observer is active; panics with its state and
/// reason otherwise.
async fn await_active(handle: &ObserverHandle, function_id: &str) {
    match tokio::time::timeout(Duration::from_secs(10), handle.ready()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => panic!("observer of {function_id} failed: {e}"),
        Err(_) => panic!(
            "observer of {function_id} not active after 10s: {:?} ({:?})",
            handle.state(),
            handle.reason()
        ),
    }
}

pub async fn call(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, iii_sdk::errors::Error> {
    iii.call(CallRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(30_000),
    })
    .await
}

pub async fn state_get(iii: &IIIClient, scope: &str, key: &str) -> Value {
    call(iii, "state::get", json!({ "scope": scope, "key": key }))
        .await
        .expect("state::get")
}

pub async fn state_set(iii: &IIIClient, scope: &str, key: &str, value: Value) {
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
        "point": "pre_call",
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

/// Run `f` against a freshly booted stack; skip when the engine or the state
/// worker binary is unavailable.
///
/// The boot lock is held for the WHOLE test, not just the spawn: each
/// engine-backed test runs its own engine, and approval-gate depends on the
/// configuration worker being responsive at boot. Serializing the tests
/// keeps that worker from being starved under parallel load (the suite is
/// local-only — it self-skips in CI, where neither binary is provided).
pub async fn with_stack<F, Fut>(opts: BootOpts, f: F)
where
    F: FnOnce(TestStack) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let _guard = boot_lock().await;
    let Some(engine) = spawn_engine().await else {
        eprintln!("skipping: {}", stack_unavailable().unwrap_or_default());
        return;
    };
    let stack = boot(&engine, opts).await;
    f(stack).await;
    // `engine` drops here — isolated state cleaned up with the temp dir.
}
