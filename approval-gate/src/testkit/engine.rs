//! Engine-backed test bootstrap (llm-router / integration.rs pattern).
//!
//! The stack is an engine (`III_ENGINE_BIN`, or `iii` on PATH) with its
//! `configuration` worker, plus the standalone `state` worker of this repo
//! (`III_STATE_BIN`, a built `state/target/debug/state`): the engine hosts no
//! state worker of its own. Tests skip when either binary is unavailable and
//! fail when both are there and the stack does not come up.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iii_sdk::{
    register_worker, CallRequest, Capture, IIIClient, InitOptions, ObserverHandle,
    RegisterFunction, Source,
};
use serde_json::{json, Value};
use tokio::sync::{OnceCell, RwLock};

use crate::configuration::{self, ConfigCell};
use crate::events::{self, Emitter, ON_PENDING_CREATED_FN, ON_PENDING_RESOLVED_FN};
use crate::functions::{self, Deps};

pub struct Engine {
    pub url: String,
    children: Vec<std::process::Child>,
    dir: std::path::PathBuf,
}

fn stop_child(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // The engine launcher may fork; stop the isolated process group too.
        let _ = std::process::Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", child.id())])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

impl Drop for Engine {
    fn drop(&mut self) {
        for child in self.children.iter_mut().rev() {
            stop_child(child);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A path-like binary made absolute: the stack's processes start in its own
/// directory, which would re-base a relative one. A bare name stays a `PATH`
/// lookup.
fn anchored(bin: std::path::PathBuf) -> std::path::PathBuf {
    if bin.components().count() > 1 {
        std::path::absolute(&bin).unwrap_or(bin)
    } else {
        bin
    }
}

pub fn engine_bin() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("III_ENGINE_BIN") {
        return Some(anchored(p.into()));
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
    std::env::var_os("III_STATE_BIN").map(|p| anchored(p.into()))
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

/// A working directory of its own for one stack: the process id, a
/// per-process sequence number and the wall clock, so no two stacks (in one
/// test process or across runs) share the directory one of them removes on
/// drop.
fn stack_dir() -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "approval-gate-it-{}-{seq}-{nanos}",
        std::process::id()
    ))
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn worker_bin(env: &str, worker: &str) -> std::path::PathBuf {
    std::env::var_os(env)
        .map(|p| anchored(p.into()))
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join(worker)
                .join("target/debug")
                .join(worker)
        })
}

fn spawn_logged(
    dir: &std::path::Path,
    name: &str,
    bin: &std::path::Path,
    args: &[&str],
) -> std::process::Child {
    let log = std::fs::File::create(dir.join(format!("{name}.log"))).expect("create worker log");
    let mut command = std::process::Command::new(bin);
    command
        .args(args)
        .current_dir(dir)
        // The engine and every SDK client write an `iii ps` marker under $HOME/.iii/run.
        .env("HOME", dir)
        .env("III_NAMESPACE", "default")
        .env("III_COMPOSE_DIR", dir)
        .stdout(log.try_clone().expect("clone log"))
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .spawn()
        .unwrap_or_else(|e| panic!("start supplied {name} {}: {e}", bin.display()))
}

impl Engine {
    fn diagnostics(&self) -> String {
        ["engine", "state", "queue", "session-manager", "harness"]
            .iter()
            .map(|name| {
                format!(
                    "{name}: {}",
                    std::fs::read_to_string(self.dir.join(format!("{name}.log")))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn assert_alive(&mut self) {
        for child in &mut self.children {
            if let Some(status) = child.try_wait().expect("check worker process") {
                panic!(
                    "supplied integration process exited {status}:\n{}",
                    self.diagnostics()
                );
            }
        }
    }
}

pub async fn spawn_engine() -> Option<Engine> {
    let bin = engine_bin()?;
    let port = free_port();
    let dir = stack_dir();
    std::fs::create_dir_all(dir.join("configuration"))
        .expect("create isolated configuration directory");
    let config = format!(
        r#"workers:
  - name: iii-worker-manager
    config:
      host: 127.0.0.1
      port: {port}
  - name: configuration
    config:
      adapter:
        name: fs
        config:
          directory: {dir}/configuration
      ttl_seconds: 0
"#,
        dir = dir.display()
    );
    let config_path = dir.join("config.yaml");
    std::fs::write(&config_path, config).expect("write engine configuration");
    let child = spawn_logged(
        &dir,
        "engine",
        &bin,
        &[
            "--no-update-check",
            "--config",
            config_path.to_str().unwrap(),
        ],
    );
    let mut engine = Engine {
        url: format!("ws://127.0.0.1:{port}"),
        children: vec![child],
        dir,
    };
    let probe = Arc::new(register_worker(&engine.url, InitOptions::default()));
    // Shuts the probe down on every exit, including a failed readiness check.
    let _probe_guard = ClientGuard(Some(probe.clone()));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        engine.assert_alive();
        if probe
            .call(CallRequest {
                function_id: "configuration::ensure".into(),
                payload: json!({"id":"readiness_probe", "name":"Readiness", "description":"Test readiness", "schema":{}, "initial_value":{}}),
                action: None,
                timeout_ms: Some(500),
            })
            .await
            .is_ok()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "supplied engine failed readiness:\n{}",
            engine.diagnostics()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let state_seed = engine.dir.join("state.yaml");
    std::fs::write(
        &state_seed,
        "adapter:\n  name: kv\n  config:\n    store_method: in_memory\n",
    )
    .expect("write the state worker seed");
    engine.children.push(spawn_logged(
        &engine.dir,
        "state",
        &worker_bin("III_STATE_BIN", "state"),
        &[
            "--url",
            &engine.url,
            "--config",
            state_seed.to_str().unwrap(),
        ],
    ));
    loop {
        engine.assert_alive();
        if probe
            .call(CallRequest {
                function_id: "state::get".into(),
                payload: json!({"scope":"readiness", "key":"probe"}),
                action: None,
                timeout_ms: Some(500),
            })
            .await
            .is_ok()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "real state worker failed readiness:\n{}",
            engine.diagnostics()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
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
    _harness_workers: TestWorkers,
}

impl Drop for TestStack {
    fn drop(&mut self) {
        // Keep the worker lifetime tied to the whole stack, including when an
        // async test captures only its public client field. Joins the
        // connection thread, which removes this client's `iii ps` marker from
        // the test process's $HOME/.iii/run.
        self.iii.shutdown();
    }
}

#[derive(Default)]
struct TestWorkers(Vec<std::process::Child>);
impl Drop for TestWorkers {
    fn drop(&mut self) {
        for child in self.0.iter_mut().rev() {
            stop_child(child);
        }
    }
}

/// Shuts the client down on drop unless emptied.
struct ClientGuard(Option<Arc<IIIClient>>);

impl Drop for ClientGuard {
    fn drop(&mut self) {
        if let Some(iii) = self.0.take() {
            iii.shutdown();
        }
    }
}

async fn wait_for_function(iii: &IIIClient, function_id: &str, payload: Value) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if iii
            .call(CallRequest {
                function_id: function_id.into(),
                payload: payload.clone(),
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
    // Until `TestStack` owns the client, a failed step must still remove its marker.
    let mut guard = ClientGuard(Some(iii.clone()));

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

    if !opts.real_harness {
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
    }

    let created: CallLog = Arc::default();
    {
        let log = created.clone();
        iii.register_function(
            "recorder::on_created",
            RegisterFunction::new_async(move |req: Value| {
                let log = log.clone();
                async move {
                    log_push(&log, req["output"].clone());
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
                    log_push(&log, req["output"].clone());
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
            Source::function(function_id).capture([Capture::Output]),
            recorder,
        );
        await_active(&handle, function_id).await;
    }

    let gate_observer = configuration::observe_hook(&iii);

    // The typed, re-fetching config-change handler + its observer of
    // `configuration::on-updated` (mirrors the production boot order: last).
    configuration::observe_config_changes(&iii, config.clone());

    let mut harness_workers = TestWorkers::default();
    if opts.real_harness {
        harness_workers.0.push(spawn_logged(
            &engine.dir,
            "queue",
            &worker_bin("III_QUEUE_BIN", "queue"),
            &["--url", &engine.url],
        ));
        assert!(
            wait_for_function(&iii, "engine::queue::list_topics", json!({})).await,
            "queue worker did not become ready: {}",
            engine.diagnostics()
        );
        harness_workers.0.push(spawn_logged(
            &engine.dir,
            "session-manager",
            &worker_bin("III_SESSION_MANAGER_BIN", "session-manager"),
            &["--url", &engine.url],
        ));
        assert!(
            wait_for_function(&iii, "session::list", json!({})).await,
            "session-manager worker did not become ready: {}",
            engine.diagnostics()
        );
        let harness = std::env::var_os("CARGO_BIN_EXE_harness")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| worker_bin("III_HARNESS_BIN", "harness"));
        harness_workers.0.push(spawn_logged(
            &engine.dir,
            "harness",
            &harness,
            &["--url", &engine.url],
        ));
        assert!(
            wait_for_function(&iii, "harness::status", json!({"session_id":"__probe__"})).await,
            "harness worker did not become ready: {}",
            std::fs::read_to_string(engine.dir.join("harness.log")).unwrap_or_default()
        );
        // The gate's observer of `harness::hook::on-pre-call` stays pending
        // until the harness registers its hook function.
        await_active(&gate_observer, configuration::HOOK_ON_PRE_CALL_FN).await;
    }

    // A failed step above unwinds through `guard` (client shut down) and
    // `harness_workers` (children reaped); from here `TestStack` owns both.
    guard.0 = None;
    TestStack {
        iii,
        deps,
        config,
        harness_calls,
        created,
        resolved,
        _harness_workers: harness_workers,
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

#[cfg(test)]
mod tests {
    use super::{anchored, stack_dir};

    #[test]
    fn a_relative_binary_survives_the_stacks_working_directory() {
        assert_eq!(anchored("state".into()), std::path::Path::new("state"));
        let relative = anchored("../state/target/debug/state".into());
        assert!(relative.is_absolute(), "{}", relative.display());
        assert!(relative.ends_with("state/target/debug/state"));
    }

    #[test]
    fn every_stack_gets_its_own_directory() {
        let dirs: std::collections::HashSet<_> = (0..64).map(|_| stack_dir()).collect();
        assert_eq!(dirs.len(), 64);
    }
}
