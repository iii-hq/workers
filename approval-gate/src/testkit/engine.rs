//! Engine-backed test bootstrap, booted the way iii 0.25 accepts it.
//!
//! iii 0.25 refuses `iii-state` / `configuration` inline in a `config.yaml`
//! (`[UNSUPPORTED_CONFIG_WORKERS]`), so each test writes a throwaway
//! `worker-compose.yaml` and runs `iii compose --up` on it: the managed
//! engine (on a free port) carries the built-in `configuration` worker under
//! `engine.workers` (fs adapter in the temp dir), and the repo's standalone
//! `state` worker runs as the only container (in-memory kv), like
//! harness/worker-compose.yaml declares it. No `namespace:` is set, so the
//! project's functions live in the engine's default namespace.
//!
//! Self-skips only when no `iii` binary is found (`III_ENGINE_BIN`, else
//! `iii` on PATH). Any other boot failure is a test failure, with the tail
//! of the compose log. `III_STATE_BIN` names a prebuilt `state` binary;
//! otherwise the container builds and runs `../state` with `cargo run`.
//!
//! The compose daemon and every worker it or the testkit starts run without
//! the ambient supervisor variables (`III_URL`, `III_CONFIG_NAME`, ...), and
//! the testkit's own connections use an explicit identity (no ambient
//! `III_NAMESPACE` / `III_WORKER_NAME`), so a test never joins or renames
//! anything outside its own project. The in-process approval-gate still reads
//! `III_CONFIG_NAME` (the test process environment cannot be changed safely
//! while other test threads run); tests address its entry by
//! `configuration::config_id()`, inside the isolated project.
//! `EngineOpts::with_queue` adds the repo's `queue` worker, which the real
//! `harness` worker (`III_HARNESS_BIN`, else built from `../harness`) needs.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction, WorkerIdentityMode};
use serde_json::{json, Value};
use tokio::sync::{OnceCell, RwLock};

use crate::configuration::{self, ConfigCell};
use crate::events::{self, Emitter, PENDING_CREATED, PENDING_RESOLVED};
use crate::functions::{self, Deps};

/// Boot budget: the first run may compile the `state` worker.
const BOOT_TIMEOUT: Duration = Duration::from_secs(600);

pub struct Engine {
    pub url: String,
    /// The `iii compose --up` daemon; it owns the managed engine and the
    /// `state` (and `queue`) containers.
    child: std::process::Child,
    dir: PathBuf,
    /// Whether the project runs the `queue` worker.
    queue: bool,
}

impl Drop for Engine {
    fn drop(&mut self) {
        stop_compose(&mut self.child);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Interrupt the compose daemon so it stops its engine and workers, then
/// kill whatever is left of its process group.
fn stop_compose(child: &mut std::process::Child) {
    let pid = child.id().to_string();
    let _ = std::process::Command::new("kill")
        .args(["-s", "INT", &pid])
        .stderr(std::process::Stdio::null())
        .status();
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // The daemon was spawned as its own process-group leader.
    let _ = std::process::Command::new("kill")
        .args(["-s", "KILL", "--", &format!("-{pid}")])
        .stderr(std::process::Stdio::null())
        .status();
    let _ = child.kill();
    let _ = child.wait();
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

/// What a supervisor (an enclosing `iii compose`, an agent shell) sets for
/// its own worker. Inherited by the test project, `III_URL` makes compose
/// join that engine instead of starting its own (DAEMON_ALREADY_SERVING),
/// and the others would rename or re-home the children's identities.
const AMBIENT_ENV: &[&str] = &[
    "III_URL",
    "III_NAMESPACE",
    "III_CONFIG_NAME",
    "III_WORKER_NAME",
    "III_COMPOSE_NAMESPACE",
    "III_COMPOSE_FILE",
    "III_COMPOSE_DIR",
];

/// Strip [`AMBIENT_ENV`] from a child of the test project.
fn isolate_from_ambient(command: &mut std::process::Command) -> &mut std::process::Command {
    for key in AMBIENT_ENV {
        command.env_remove(key);
    }
    command
}

/// A testkit connection (readiness probe, test stack) to the project's
/// engine. Explicit identity: the SDK's managed default would adopt an
/// ambient `III_NAMESPACE` / `III_WORKER_NAME` and leave the project's
/// default namespace, where `state`, `queue` and the harness serve.
fn connect(url: &str) -> IIIClient {
    register_worker(
        url,
        InitOptions {
            identity: WorkerIdentityMode::Explicit,
            ..Default::default()
        },
    )
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

/// A repo worker crate next to this one (`state`, `queue`, `harness`).
fn sibling_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(name);
    dir.canonicalize().unwrap_or(dir)
}

/// The `scripts.run` line of a repo worker container: the binary `bin_env`
/// names when set, otherwise `cargo run` on the sibling crate.
fn worker_command(name: &str, bin_env: &str, url: &str, seed: Option<&Path>) -> String {
    let config = seed
        .map(|seed| format!(" --config {}", seed.display()))
        .unwrap_or_default();
    match std::env::var_os(bin_env) {
        Some(bin) => format!("{} --url {url}{config}", Path::new(&bin).display()),
        None => format!(
            "cargo run --quiet --manifest-path {} --bin {name} -- --url {url}{config}",
            sibling_dir(name).join("Cargo.toml").display(),
        ),
    }
}

/// One `containers:` entry for the sibling worker `name`.
fn worker_container(name: &str, dir: &Path, run: &str) -> String {
    format!(
        "  {name}:\n    worker: path://{worker}\n    working_dir: {dir}\n    required: true\n    environment:\n      RUST_LOG: warn\n    scripts:\n      run: {run}\n",
        worker = sibling_dir(name).display(),
        dir = dir.display(),
    )
}

fn compose_file(port: u16, dir: &Path, containers: &str) -> String {
    format!(
        r#"startup_timeout: 10m
stop_timeout: 10s

engine:
  url: ws://127.0.0.1:{port}
  workers:
    configuration:
      adapter:
        name: fs
        config:
          directory: {dir}/configuration
      ttl_seconds: 0

containers:
{containers}"#,
        dir = dir.display(),
    )
}

fn log_tail(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let tail = lines[lines.len().saturating_sub(40)..].join("\n");
    if tail.trim().is_empty() {
        format!("(empty log {})", path.display())
    } else {
        tail
    }
}

/// What a test engine carries besides `configuration` and `state`.
#[derive(Clone, Copy, Debug, Default)]
pub struct EngineOpts {
    /// Also run the repo's `queue` worker (the real harness needs it).
    pub queue: bool,
}

impl EngineOpts {
    pub fn with_queue() -> Self {
        Self { queue: true }
    }
}

/// Boot an isolated compose project (managed engine + `configuration` +
/// `state`). `None` only when no `iii` binary exists; any other failure
/// panics with the compose log tail.
pub async fn spawn_engine() -> Option<Engine> {
    spawn_engine_with(EngineOpts::default()).await
}

/// [`spawn_engine`] plus what `opts` asks for.
pub async fn spawn_engine_with(opts: EngineOpts) -> Option<Engine> {
    let bin = engine_bin()?;
    let port = free_port();
    // Unique per process, boot and instant: the testkit feature has no uuid
    // dependency (uuid is dev-only), so a counter + nanos stand in.
    static BOOT_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = BOOT_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!(
        "approval-gate-it-{}-{seq}-{nanos}",
        std::process::id(),
    ));
    // The configuration worker's fs adapter writes one YAML file per id
    // under this directory; create it up front so register/get never race
    // a missing path (approval-gate treats configuration as a required boot
    // dependency).
    std::fs::create_dir_all(dir.join("configuration")).expect("create test engine dir");

    let url = format!("ws://127.0.0.1:{port}");
    let seed = dir.join("state-config.yaml");
    std::fs::write(
        &seed,
        "adapter:\n  name: kv\n  config:\n    store_method: in_memory\n",
    )
    .expect("write state seed config");
    let mut containers = worker_container(
        "state",
        &dir,
        &worker_command("state", "III_STATE_BIN", &url, Some(&seed)),
    );
    if opts.queue {
        containers.push_str(&worker_container(
            "queue",
            &dir,
            &worker_command("queue", "III_QUEUE_BIN", &url, None),
        ));
    }
    let compose = compose_file(port, &dir, &containers);
    let compose_path = dir.join("worker-compose.yaml");
    std::fs::File::create(&compose_path)
        .and_then(|mut f| f.write_all(compose.as_bytes()))
        .expect("write worker-compose.yaml");

    // A file, not a pipe: compose prints with bare `println!`, and nobody
    // drains a pipe during the test.
    let log_path = dir.join("compose.log");
    let log = std::fs::File::create(&log_path).expect("create compose log");
    let log_err = log.try_clone().expect("clone compose log handle");
    let mut command = std::process::Command::new(&bin);
    // Without this, an ambient III_URL makes compose join that engine
    // instead of starting its own (DAEMON_ALREADY_SERVING).
    isolate_from_ambient(&mut command)
        .arg("--no-update-check")
        .arg("compose")
        .arg("--up")
        .arg("--file")
        .arg(&compose_path)
        .current_dir(&dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(log_err));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    let child = command
        .spawn()
        .unwrap_or_else(|e| panic!("spawn `{} compose --up`: {e}", bin.display()));
    // From here on, dropping `engine` (a panic included) stops the project.
    let mut engine = Engine {
        url,
        child,
        dir,
        queue: opts.queue,
    };

    let probe = connect(&engine.url);
    let deadline = Instant::now() + BOOT_TIMEOUT;
    loop {
        if let Ok(Some(status)) = engine.child.try_wait() {
            probe.shutdown();
            panic!(
                "iii compose exited during boot ({status}); compose log:\n{}",
                log_tail(&log_path)
            );
        }
        // Ready = the engine core answers, the configuration worker serves
        // (any response, NOT_FOUND included, means it is up), the state
        // container answers `state::get` and, when asked for, the queue
        // container has registered `queue::define`.
        let core_ready = probe
            .trigger(TriggerRequest {
                function_id: "engine::workers::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(1000),
            })
            .await
            .is_ok();
        let config_ready = core_ready
            && match probe
                .trigger(TriggerRequest {
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
        let state_ready = config_ready
            && probe
                .trigger(TriggerRequest {
                    function_id: "state::get".into(),
                    payload: json!({ "scope": "__readiness_probe__", "key": "ready" }),
                    action: None,
                    timeout_ms: Some(1000),
                })
                .await
                .is_ok();
        let queue_ready =
            state_ready && (!opts.queue || function_listed(&probe, "queue::define").await);
        if queue_ready {
            break;
        }
        if Instant::now() >= deadline {
            probe.shutdown();
            panic!(
                "iii compose project not ready within {}s (engine core: {core_ready}, configuration: {config_ready}, state: {state_ready}, queue wanted: {}); compose log:\n{}",
                BOOT_TIMEOUT.as_secs(),
                opts.queue,
                log_tail(&log_path)
            );
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    probe.shutdown();

    Some(engine)
}

/// Whether `function_id` is registered on the engine.
async fn function_listed(probe: &IIIClient, function_id: &str) -> bool {
    match probe
        .trigger(TriggerRequest {
            function_id: "engine::functions::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(1000),
        })
        .await
    {
        Ok(list) => list.to_string().contains(&format!("\"{function_id}\"")),
        Err(_) => false,
    }
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
    /// modeling a post-trigger hook that parks the same call again.
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
    /// `approval::pending-created` deliveries.
    pub created: CallLog,
    /// `approval::pending-resolved` deliveries.
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

/// The real `harness` worker binary: `III_HARNESS_BIN`, else
/// `CARGO_BIN_EXE_harness`, else built once per test process from the
/// sibling `harness` crate (its own target dir). A failed build is a test
/// failure, never a skip.
pub fn harness_bin() -> PathBuf {
    static BIN: std::sync::OnceLock<Result<PathBuf, String>> = std::sync::OnceLock::new();
    BIN.get_or_init(|| {
        for key in ["III_HARNESS_BIN", "CARGO_BIN_EXE_harness"] {
            if let Some(bin) = std::env::var_os(key).filter(|bin| !bin.is_empty()) {
                return Ok(PathBuf::from(bin));
            }
        }
        let dir = sibling_dir("harness");
        let target = dir.join("target");
        let output = std::process::Command::new("cargo")
            .args(["build", "--quiet", "--bin", "harness", "--manifest-path"])
            .arg(dir.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(&target)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("run `cargo build` for the harness worker: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "`cargo build` of the harness worker failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(target
            .join("debug")
            .join(format!("harness{}", std::env::consts::EXE_SUFFIX)))
    })
    .clone()
    .unwrap_or_else(|e| panic!("{e}"))
}

/// Start the real harness worker against `engine`, isolated from the
/// ambient supervisor environment, logging to `<engine dir>/harness.log`.
fn spawn_harness_worker(bin: &Path, engine: &Engine) -> std::process::Child {
    let log = std::fs::File::create(engine.dir.join("harness.log")).expect("create harness log");
    let log_err = log.try_clone().expect("clone harness log handle");
    let mut command = std::process::Command::new(bin);
    isolate_from_ambient(&mut command)
        .arg("--url")
        .arg(&engine.url)
        .current_dir(&engine.dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(log_err));
    command
        .spawn()
        .unwrap_or_else(|e| panic!("spawn harness worker {}: {e}", bin.display()))
}

async fn wait_for_harness(iii: &IIIClient) -> bool {
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        if iii
            .trigger(TriggerRequest {
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
    let iii = Arc::new(connect(&engine.url));

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

    let sets = events::register_trigger_types(&iii);
    let deps = Arc::new(Deps {
        iii: iii.clone(),
        sink: Arc::new(Emitter::new(sets, iii.clone())),
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
    // Old-style consumers: the trigger types stay served on 0.25 engines.
    #[allow(deprecated)]
    iii.register_trigger(RegisterTriggerInput::new(
        PENDING_CREATED.to_string(),
        "recorder::on_created".to_string(),
        json!({}),
    ))
    .expect("bind pending_created");
    #[allow(deprecated)]
    iii.register_trigger(RegisterTriggerInput::new(
        PENDING_RESOLVED.to_string(),
        "recorder::on_resolved".to_string(),
        json!({}),
    ))
    .expect("bind pending_resolved");

    configuration::bind_hook(&iii);

    // The typed, re-fetching config-change handler + its `configuration`
    // trigger binding (mirrors the production boot order: last).
    configuration::register_config_trigger(&iii, config.clone())
        .expect("register configuration change trigger");

    let harness_child = if opts.real_harness {
        assert!(
            engine.queue,
            "the real harness worker needs the queue worker: spawn the engine with EngineOpts::with_queue()"
        );
        let bin = tokio::task::spawn_blocking(harness_bin)
            .await
            .unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()));
        let mut child = spawn_harness_worker(&bin, engine);
        if !wait_for_harness(&iii).await {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "harness worker did not become ready; harness log:\n{}",
                log_tail(&engine.dir.join("harness.log"))
            );
        }
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

pub async fn call(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, iii_sdk::errors::Error> {
    iii.trigger(TriggerRequest {
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
        "point": "pre_trigger",
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
///
/// The boot lock is held for the WHOLE test, not just the spawn: each
/// engine-backed test runs its own engine, and approval-gate now depends
/// on the configuration worker being responsive at boot. Serializing the
/// tests keeps that worker from being starved under parallel load (the
/// suite is local-only — it self-skips in CI where `iii` is absent).
pub async fn with_stack<F, Fut>(opts: BootOpts, f: F)
where
    F: FnOnce(TestStack) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    with_stack_using(EngineOpts::default(), opts, f).await;
}

/// [`with_stack`] on an engine carrying what `engine_opts` asks for (the
/// real-harness tests need [`EngineOpts::with_queue`]). Skips only when no
/// `iii` binary exists.
pub async fn with_stack_using<F, Fut>(engine_opts: EngineOpts, opts: BootOpts, f: F)
where
    F: FnOnce(TestStack) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let _guard = boot_lock().await;
    let Some(engine) = spawn_engine_with(engine_opts).await else {
        eprintln!("skipping: no iii engine (set III_ENGINE_BIN or put `iii` on PATH)");
        return;
    };
    let stack = boot(&engine, opts).await;
    f(stack).await;
    // `engine` drops here — isolated state cleaned up with the temp dir.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_of_the_test_project_drop_the_ambient_supervisor_env() {
        let mut command = std::process::Command::new("iii");
        isolate_from_ambient(&mut command);
        let removed: Vec<_> = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect();
        for key in ["III_URL", "III_CONFIG_NAME", "III_COMPOSE_NAMESPACE"] {
            assert!(
                removed.iter().any(|k| k == key),
                "{key} must not reach the compose daemon or the harness worker"
            );
        }
    }

    #[test]
    fn the_compose_file_carries_the_queue_only_when_asked() {
        let dir = Path::new("/tmp/approval-gate-it");
        let url = "ws://127.0.0.1:4242";
        let state = worker_container(
            "state",
            dir,
            &worker_command("state", "III_STATE_BIN", url, None),
        );
        let plain = compose_file(4242, dir, &state);
        assert!(plain.contains("url: ws://127.0.0.1:4242"));
        assert!(plain.contains("\n  state:\n"));
        assert!(!plain.contains("\n  queue:\n"));

        let queue = worker_container(
            "queue",
            dir,
            &worker_command("queue", "III_QUEUE_BIN", url, None),
        );
        let with_queue = compose_file(4242, dir, &format!("{state}{queue}"));
        assert!(with_queue.contains("\n  queue:\n"));
        assert!(with_queue.contains(&format!("--url {url}")));
    }
}
